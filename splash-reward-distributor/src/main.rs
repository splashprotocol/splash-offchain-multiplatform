mod config;
mod emission;
mod engine;
mod entity_index;
mod events;
mod indexer;
mod onchain;
mod pipeline;
mod positions;

use crate::config::AppConfig;
use async_primitives::beacon::Beacon;
use cardano_chain_sync::atomic_flow::atomic_block_flow;
use cardano_chain_sync::cache::LedgerCacheRocksDB;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_explorer::AnyExplorer;
use clap::Parser;
use futures::stream::FuturesUnordered;
use log::info;
use spectrum_streaming::run_stream;
use splash_dao_offchain::deployment::{
    DeployedValidators as DaoValidators, ProtocolDeployment as DaoDeployment,
};
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
        std::fs::read_to_string(args.dao_deployment_path).expect("Cannot load DAO deployment file");
    let dao_validators: DaoValidators =
        serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting LP indexer ..");

    let explorer = AnyExplorer::new(&config.explorer, config.network_id)
        .await
        .expect("Explorer initialization failed");

    let dao_protocol_deployment = DaoDeployment::unsafe_pull(dao_validators, &explorer).await;

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

    // let ip_addr = IpAddr::from_str(&*args.host).expect("Invalid host address");
    // let bind_addr = SocketAddr::new(ip_addr, args.port);

    let processes = FuturesUnordered::new();

    let flow_driver_handle = tokio::spawn(flow_driver.run());
    processes.push(flow_driver_handle);

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
    /// Path to the DAO deployment JSON configuration file .
    #[arg(long)]
    dao_deployment_path: String,
    #[arg(long)]
    dao_tokens_path: String,
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
