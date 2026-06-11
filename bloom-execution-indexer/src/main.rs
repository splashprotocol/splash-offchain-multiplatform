use actix_web::dev::ServerHandle;
use async_primitives::beacon::Beacon;
use bloom_execution_indexer::config::AppConfig;
use bloom_execution_indexer::handler::TxHandler;
use bloom_execution_indexer::http_api::build_api_server;
use bloom_execution_indexer::store::rocks::RocksIndex;
use bloom_offchain_cardano::event_sink::tx_view::TxViewMut;
use cardano_chain_sync::cache::LedgerCacheRocksDB;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_chain_sync::event_source::ledger_transactions;
use clap::Parser;
use futures::stream::StreamExt;
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain::tracing::Tracing;
use spectrum_streaming::run_stream;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = AppArgs::parse();
    let raw_config = std::fs::File::open(args.config_path).expect("cannot load configuration file");
    let config: AppConfig = serde_json::from_reader(raw_config).expect("invalid configuration file");
    config
        .validate()
        .expect("invalid execution indexer configuration");

    let chain_sync_cache = Arc::new(Mutex::new(LedgerCacheRocksDB::new(
        config.chain_sync.db_path.clone(),
    )));
    let state_synced = Beacon::relaxed(false);
    let rollback_in_progress = Beacon::strong(false);
    let startup_complete = Arc::new(AtomicBool::new(false));
    let index = RocksIndex::open(config.index_db_path.clone());

    let chain_sync = ChainSyncClient::init(
        Arc::clone(&chain_sync_cache),
        config.node.path.clone(),
        config.node.magic,
        config.chain_sync.starting_point.clone(),
    )
    .await
    .expect("chain-sync initialization failed");

    let ledger_stream = Box::pin(ledger_transactions(
        chain_sync_cache,
        chain_sync_stream(chain_sync, state_synced.clone()),
        config.chain_sync.disable_rollbacks_until,
        None,
        rollback_in_progress,
    ))
    .await
    .map(|ev| ev.map(TxViewMut::from));

    let handler = TxHandler::new(
        Tracing::attach(index.clone()),
        config.network_id,
        config.tracked_limit_order_script_hashes,
    );
    let handlers: Vec<Box<dyn EventHandler<_> + Send>> = vec![Box::new(handler)];
    let process_ledger_events_stream = process_events(ledger_stream, handlers);

    let ip_addr = IpAddr::from_str(&config.http.host).expect("invalid http.host");
    let bind_addr = SocketAddr::new(ip_addr, config.http.port);
    let server = build_api_server(
        index,
        state_synced.clone(),
        Arc::clone(&startup_complete),
        bind_addr,
    )
    .await
    .expect("error setting up api server");
    let server_handle = server.handle();

    startup_complete.store(true, Ordering::Release);

    let stream_task = tokio::spawn(run_stream(process_ledger_events_stream));
    let server_task = tokio::spawn(server);
    let shutdown_task = tokio::spawn(shutdown(server_handle));

    tokio::select! {
        result = stream_task => result.expect("ledger stream task failed"),
        result = server_task => result.expect("http server task failed").expect("http server failed"),
        _ = shutdown_task => {},
    }
}

async fn shutdown(server_handle: ServerHandle) {
    tokio::signal::ctrl_c().await.expect("ctrl-c handler failed");
    server_handle.stop(true).await;
}

#[derive(Parser)]
#[command(name = "bloom-execution-indexer")]
#[command(author = "Spectrum Labs")]
#[command(version = "0.1.0")]
#[command(about = "Ledger-only batcher execution metrics indexer", long_about = None)]
struct AppArgs {
    #[arg(long, short)]
    config_path: String,
}
