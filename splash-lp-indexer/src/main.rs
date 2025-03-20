use crate::config::AppConfig;
use crate::context::Context;
use crate::feed::event::ExportAccountEvent;
use crate::feed::event_publisher::EventPublisher;
use crate::gauge_index::GaugeIndexDB;
use crate::http_api::build_api_server;
use crate::pipeline::{log_events, process_mature_events};
use crate::position_db::PositionDB;
use async_primitives::beacon::Beacon;
use bloom_offchain_cardano::validation_rules::ValidationRules;
use cardano_chain_sync::atomic_flow::atomic_block_flow;
use cardano_chain_sync::cache::LedgerCacheRocksDB;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_explorer::{AnyExplorer, Maestro};
use clap::Parser;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use log::info;
use rdkafka::producer::FutureProducer;
use rdkafka::ClientConfig;
use spectrum_offchain_cardano::deployment::{DeployedValidators, ProtocolDeployment};
use spectrum_offchain_cardano::persistent_index::IndexRocksDB;
use spectrum_streaming::run_stream;
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing_subscriber::fmt::Subscriber;

mod account;
mod config;
mod constants;
mod context;
mod feed;
mod gauge_index;
mod http_api;
mod onchain;
mod pipeline;
mod position_db;
mod tx_view;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    let raw_deployment = std::fs::read_to_string(args.deployment_path).expect("Cannot load deployment file");
    let deployment: DeployedValidators =
        serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

    let raw_validation_rules =
        std::fs::read_to_string(args.validation_rules_path).expect("Cannot load bounds file");
    let validation_rules: ValidationRules =
        serde_json::from_str(&raw_validation_rules).expect("Invalid bounds file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting LP indexer ..");

    let explorer = AnyExplorer::new(&config.explorer, config.network_id)
        .await
        .expect("Explorer initialization failed");

    let protocol_deployment = ProtocolDeployment::unsafe_pull(deployment, &explorer).await;

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
        Box::pin(chain_sync_stream(chain_sync, state_synced)),
        chain_sync_cache,
    );

    let utxo_index = IndexRocksDB::new(config.utxo_index_db_path);
    let position_db = PositionDB::new(config.accounts_db_path);
    let filter = HashSet::from([
        protocol_deployment.balance_fn_pool_v1.hash,
        protocol_deployment.balance_fn_pool_v2.hash,
        protocol_deployment.const_fn_pool_v1.hash,
        protocol_deployment.const_fn_pool_v2.hash,
        protocol_deployment.royalty_pool.hash,
        protocol_deployment.stable_fn_pool_t2t.hash,
    ]);
    let cx = Context {
        deployment: protocol_deployment,
        pool_validation: validation_rules.pool,
    };

    let ip_addr = IpAddr::from_str(&*args.host).expect("Invalid host address");
    let bind_addr = SocketAddr::new(ip_addr, args.port);
    let server = build_api_server(position_db.clone(), bind_addr)
        .await
        .expect("Error setting up api server")
        .map(|r| r.unwrap());

    let kafka = ClientConfig::new()
        .set("bootstrap.servers", &config.bootstrap_servers)
        .create::<FutureProducer>()
        .expect("Failed to create kafka producer");
    let publisher =
        EventPublisher::<ExportAccountEvent, _>::new(position_db.clone(), kafka, config.events_export_topic);

    let gauges_db = GaugeIndexDB::new(config.gauges_db_path);

    let processes = FuturesUnordered::new();

    let flow_driver_handle = tokio::spawn(flow_driver.run());
    processes.push(flow_driver_handle);

    let log_events_handle = tokio::spawn(log_events(
        block_events,
        position_db.clone(),
        cx,
        utxo_index,
        gauges_db,
        filter,
    ));
    processes.push(log_events_handle);

    let process_mature_events_handle = tokio::spawn(process_mature_events(
        position_db,
        config.confirmation_delay_blocks,
    ));
    processes.push(process_mature_events_handle);

    let export_events_handle = tokio::spawn(publisher.run());
    processes.push(export_events_handle);

    let server_handle = tokio::spawn(server);
    processes.push(server_handle);

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
    /// Path to the deployment JSON configuration file .
    #[arg(long, short)]
    deployment_path: String,
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
