use async_primitives::beacon::Beacon;
use bloom_offchain_cardano::validation_rules::ValidationRules;
use cardano_chain_sync::atomic_flow::atomic_block_flow;
use cardano_chain_sync::cache::LedgerCacheRocksDB;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_explorer::AnyExplorer;
use clap::Parser;
use cml_crypto::ScriptHash;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use log::info;
use rdkafka::producer::FutureProducer;
use rdkafka::ClientConfig;
use spectrum_cardano_lib::time::posix_to_slot;
use spectrum_offchain_cardano::deployment::{
    DeployedValidators as DexValidators, ProtocolDeployment as DexDeployment,
};
use spectrum_offchain_cardano::persistent_index::IndexRocksDB;
use spectrum_streaming::run_stream;
use splash_dao_offchain::constants::time::EPOCH_LEN;
use splash_dao_offchain::deployment::{
    CompleteDeployment as DaoDeployment, DeploymentProgress as DaoDeploymentProgress,
    ProtocolDeployment as DaoProtocolDeployment,
};
use splash_lp_index::config::AppConfig;
use splash_lp_index::context::RuntimeContext;
use splash_lp_index::feed::event::ExportAccountPositionEvent;
use splash_lp_index::feed::event_publisher::EventPublisher;
use splash_lp_index::http_api::build_api_server;
use splash_lp_index::pipeline::{event_pipeline, process_mature_events};
use splash_lp_index::position_db::PositionDB;
use splash_lp_index::ve_index::VoteEscrowDB;
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing_subscriber::fmt::Subscriber;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    let raw_deployment =
        std::fs::read_to_string(args.dex_deployment_path).expect("Cannot load DEX deployment file");
    let dex_validators: DexValidators =
        serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

    let raw_deployment =
        std::fs::read_to_string(args.dao_deployment_path).expect("Cannot load DAO deployment file");
    let dao_deployment: DaoDeploymentProgress =
        serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
    let dao_deployment = DaoDeployment::try_from((dao_deployment, config.network_id)).unwrap();

    let raw_validation_rules =
        std::fs::read_to_string(args.validation_rules_path).expect("Cannot load bounds file");
    let validation_rules: ValidationRules =
        serde_json::from_str(&raw_validation_rules).expect("Invalid bounds file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting LP indexer ..");

    let explorer = AnyExplorer::new(&config.explorer, config.network_id)
        .await
        .expect("Explorer initialization failed");

    let dex_protocol_deployment = DexDeployment::unsafe_pull(dex_validators, &explorer).await;
    let dao_protocol_deployment =
        DaoProtocolDeployment::unsafe_pull(dao_deployment.deployed_validators, &explorer).await;

    let chain_sync_cache = Arc::new(Mutex::new(LedgerCacheRocksDB::new(config.chain_sync.db_path)));
    let chain_sync = ChainSyncClient::init(
        Arc::clone(&chain_sync_cache),
        config.node.path.clone(),
        config.node.magic,
        config.chain_sync.starting_point,
    )
    .await
    .expect("ChainSync initialization failed");

    let state_synced = Beacon::relaxed(false);
    let (flow_driver, block_events) = atomic_block_flow(
        Box::pin(chain_sync_stream(chain_sync, state_synced.clone())),
        chain_sync_cache,
    );

    let utxo_index = IndexRocksDB::new(config.utxo_index_db_path);
    let epoch_start = posix_to_slot(dao_deployment.genesis_epoch_start_time / 1000, config.network_id);
    let position_db = PositionDB::new(
        config.accounts_db_path,
        config.confirmation_delay_slots,
        EPOCH_LEN / 1000,
        epoch_start,
    );
    let filter = HashSet::from([
        dex_protocol_deployment.balance_fn_pool_v1.hash,
        dex_protocol_deployment.balance_fn_pool_v2.hash,
        dex_protocol_deployment.const_fn_pool_v1.hash,
        dex_protocol_deployment.const_fn_pool_v2.hash,
        dex_protocol_deployment.royalty_pool.hash,
        dex_protocol_deployment.stable_fn_pool_t2t.hash,
    ]);

    let cx = RuntimeContext {
        dex_deployment: dex_protocol_deployment,
        dao_deployment: dao_protocol_deployment,
        dao_tokens: dao_deployment.minted_deployment_tokens.clone(),
        pool_validation: validation_rules.pool,
        harvest_limits: config.harvest_limits,
        splash_policy_id: ScriptHash::from_hex(&config.splash_policy_id_hex).unwrap(),
        network_id: config.network_id,
        genesis_epoch_start_time: dao_deployment.genesis_epoch_start_time.into(),
    };

    let ip_addr = IpAddr::from_str(&*args.host).expect("Invalid host address");
    let bind_addr = SocketAddr::new(ip_addr, args.port);
    let server = build_api_server(position_db.clone(), bind_addr)
        .await
        .expect("Error setting up api server");

    let server_handle = server.handle();

    //let kafka = ClientConfig::new()
    //    .set("bootstrap.servers", &config.bootstrap_servers)
    //    .create::<FutureProducer>()
    //    .expect("Failed to create kafka producer");
    //let publisher = EventPublisher::<ExportAccountPositionEvent, _>::new(
    //    position_db.clone(),
    //    kafka,
    //    config.events_export_topic,
    //);

    let gauges_db = VoteEscrowDB::new(config.gauges_db_path);

    let processes = FuturesUnordered::new();

    let flow_driver_handle = tokio::spawn(flow_driver.run(config.chain_sync.replay_from_point));
    processes.push(flow_driver_handle);

    let log_events_handle = tokio::spawn(event_pipeline(
        block_events,
        position_db.clone(),
        cx,
        utxo_index,
        gauges_db,
        filter,
    ));
    processes.push(log_events_handle);

    let process_mature_events_handle = tokio::spawn(process_mature_events(position_db));
    processes.push(process_mature_events_handle);

    //let export_events_handle = tokio::spawn(publisher.run());
    //processes.push(export_events_handle);

    let server_process_handle = tokio::spawn(server.map(|r| r.unwrap()));
    processes.push(server_process_handle);

    let shutdown = tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        server_handle.stop(true).await;
        std::process::exit(0);
    });
    processes.push(shutdown);

    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic(info);
        std::process::exit(1);
    }));

    run_stream(processes).await;
}

#[derive(Parser)]
#[command(name = "splash-lp-indexer")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Splash LP Indexer", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    /// Path to the DEX deployment JSON configuration file .
    #[arg(long)]
    dex_deployment_path: String,
    /// Path to the DAO deployment JSON configuration file .
    #[arg(long)]
    dao_deployment_path: String,
    /// Path to the bounds JSON configuration file .
    #[arg(long, short)]
    validation_rules_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
    #[arg(long)]
    host: String,
    #[arg(long)]
    port: u16,
}
