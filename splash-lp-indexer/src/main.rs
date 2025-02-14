use crate::config::AppConfig;
use crate::context::Context;
use crate::db::RocksDB;
use crate::http_api::build_api_server;
use crate::pipeline::{log_events, process_mature_events};
use async_primitives::beacon::Beacon;
use cardano_chain_sync::atomic_flow::atomic_block_flow;
use cardano_chain_sync::cache::LedgerCacheRocksDB;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_explorer::Maestro;
use clap::Parser;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use log::info;
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
mod context;
mod db;
mod feed;
mod http_api;
mod onchain;
mod pipeline;
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

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting LP indexer ..");

    let explorer = Maestro::new(config.maestro_key_path, config.network_id.into())
        .await
        .expect("Maestro instantiation failed");

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
    let rollback_in_progress = Arc::new(AtomicBool::new(false));
    let (flow_driver, block_events) = atomic_block_flow(
        Box::pin(chain_sync_stream(chain_sync, state_synced)),
        chain_sync_cache,
    );

    let index = IndexRocksDB::new(config.utxo_index_db_path);
    let db = RocksDB::new(config.accounts_db_path);
    let filter = HashSet::from([protocol_deployment.balance_fn_pool_v1.hash]);
    let cx = Context {
        deployment: protocol_deployment,
        pool_validation: config.pool_validation,
    };

    let ip_addr = IpAddr::from_str(&*args.host).expect("Invalid host address");
    let bind_addr = SocketAddr::new(ip_addr, args.port);
    let server = build_api_server(db.clone(), bind_addr)
        .await
        .expect("Error setting up api server")
        .map(|r| r.unwrap());

    let processes = FuturesUnordered::new();

    let flow_driver_handle = tokio::spawn(flow_driver.run());
    processes.push(flow_driver_handle);

    let log_events_handle = tokio::spawn(log_events(block_events, db.clone(), cx, index, filter));
    processes.push(log_events_handle);

    let process_mature_events_handle =
        tokio::spawn(process_mature_events(db, config.confirmation_delay_blocks));
    processes.push(process_mature_events_handle);

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
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
    #[arg(long)]
    host: String,
    #[arg(long)]
    port: u16,
}
