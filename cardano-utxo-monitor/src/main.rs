mod config;
mod handler;
mod index;
mod restart;
mod rollback;
mod server;

use crate::server::build_api_server;
use bloom_offchain_cardano::event_sink::tx_view::TxViewMut;
use clap::Parser;
use cml_chain::transaction::Transaction;
use derive_more::From;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use spectrum_streaming::run_stream;

use crate::config::AppConfig;
use crate::restart::{determine_restart_mode, RestartMode};
use crate::rollback::rollback_blocks;
use async_primitives::beacon::Beacon;
use cardano_chain_sync::cache::LedgerCacheRocksDB;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_chain_sync::event_source::ledger_transactions;
use cardano_mempool_sync::client::LocalTxMonitorClient;
use cardano_mempool_sync::mempool_stream;
use cml_crypto::TransactionHash;
use futures::channel::mpsc;
use log::warn;
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain_cardano::tx_tracker::new_tx_tracker_bundle;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::handler::TxHandler;
use crate::index::RocksDB;
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_mempool_sync::data::MempoolUpdate;
use futures::stream::StreamExt;
use spectrum_offchain::event_sink::event_handler::{forward_with_ref, EventHandler};
use spectrum_offchain::tracing::Tracing;
use tracing_subscriber::fmt::Subscriber;

#[tokio::main]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    let raw_config = std::fs::File::open(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_reader(raw_config).expect("Invalid configuration file");

    if config.chain_sync.replay_from_point.is_some() {
        warn!("════════════════════════════════════");
        warn!("⚠  DEPRECATED CONFIGURATION");
        warn!("════════════════════════════════════");
        warn!("'replayFromPoint' is no longer used");
        warn!("Automatic rollback is now configured via 'autoRollbackSlots'");
        warn!("Please remove 'replayFromPoint' from config");
        warn!("════════════════════════════════════");
    }

    let chain_sync_cache = Arc::new(Mutex::new(LedgerCacheRocksDB::new(
        config.chain_sync.db_path.clone(),
    )));
    let db = RocksDB::new(config.index_path.clone());
    let startup_complete = Arc::new(AtomicBool::new(false));

    let restart_mode = determine_restart_mode(
        Arc::clone(&chain_sync_cache),
        config.chain_sync.starting_point.clone(),
        config.chain_sync.auto_rollback_slots,
    )
    .await;

    let resume_point = match restart_mode {
        RestartMode::FreshStart { from } => {
            log::info!("Fresh start from {:?}", from);
            from
        }
        RestartMode::RollbackAndResume {
            current_point,
            rollback_slots: n_slots,
        } => {
            log::info!("Rolling back {} slots from {:?}", n_slots, current_point);
            match rollback_blocks(&db, Arc::clone(&chain_sync_cache), current_point, n_slots).await {
                Ok(rolled_back_point) => {
                    log::info!("Rollback complete - resuming from {:?}", rolled_back_point);
                    rolled_back_point
                }
                Err(e) => {
                    log::error!("Rollback failed: {}", e);
                    log::error!("This is critical - manual intervention may be required");
                    panic!("Rollback failed: {}", e);
                }
            }
        }
    };

    startup_complete.store(true, Ordering::Release);

    let chain_sync = ChainSyncClient::init(
        Arc::clone(&chain_sync_cache),
        config.node.path.clone(),
        config.node.magic,
        resume_point,
    )
    .await
    .expect("ChainSync initialization failed");

    let state_synced = Beacon::relaxed(false);
    let rollback_in_progress = Beacon::strong(false);

    // n2c clients:
    let mempool_sync =
        LocalTxMonitorClient::<Transaction>::connect(config.node.path.clone(), config.node.magic)
            .await
            .expect("MempoolSync initialization failed");

    let (failed_txs_snd, failed_txs_recv) = mpsc::channel(config.tx_tracker_buffer_size);
    let (confirmed_txs_snd, confirmed_txs_recv) = mpsc::channel(config.tx_tracker_buffer_size);
    let max_confirmation_delay_blocks = 6;
    let (tx_tracker_agent, tx_tracker_channel) = new_tx_tracker_bundle(
        confirmed_txs_recv,
        failed_txs_snd,
        config.tx_tracker_buffer_size,
        max_confirmation_delay_blocks,
    );

    let ledger_stream = Box::pin(ledger_transactions(
        chain_sync_cache,
        chain_sync_stream(chain_sync, state_synced.clone()),
        config.chain_sync.disable_rollbacks_until,
        None,
        rollback_in_progress,
    ))
    .await
    .map(|ev| ev.map(TxViewMut::from));

    let mempool_stream = mempool_stream(
        mempool_sync,
        tx_tracker_channel,
        failed_txs_recv,
        state_synced.clone(),
    )
    .map(|ev| ev.map(TxViewMut::from));

    let handler = TxHandler::new(Tracing::attach(db.clone()));

    let startup_complete_for_server = Arc::clone(&startup_complete);

    let handlers_ledger: Vec<Box<dyn EventHandler<LedgerTxEvent<TxViewMut>> + Send>> = vec![
        Box::new(forward_with_ref(confirmed_txs_snd, succinct_tx)),
        Box::new(handler.clone()),
    ];

    let handlers_mempool: Vec<Box<dyn EventHandler<MempoolUpdate<TxViewMut>> + Send>> =
        vec![Box::new(handler)];

    let process_ledger_events_stream = process_events(ledger_stream, handlers_ledger);
    let process_mempool_events_stream = process_events(mempool_stream, handlers_mempool);

    let ip_addr = IpAddr::from_str(&*args.host).expect("Invalid host address");
    let bind_addr = SocketAddr::new(ip_addr, args.port);
    let server = build_api_server(db, state_synced.clone(), startup_complete_for_server, bind_addr)
        .await
        .expect("Error setting up api server");

    let processes = FuturesUnordered::new();

    let process_ledger_events_stream_handle = tokio::spawn(run_stream(process_ledger_events_stream));
    processes.push(process_ledger_events_stream_handle);

    let process_mempool_events_stream_handle = tokio::spawn(run_stream(process_mempool_events_stream));
    processes.push(process_mempool_events_stream_handle);

    let tx_tracker_handle = tokio::spawn(tx_tracker_agent.run());
    processes.push(tx_tracker_handle);

    let server_handle = server.handle();
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
#[command(name = "cardano-utxo-monitor")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Splash Mempool Proxy", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    #[arg(long, short)]
    log4rs_path: String,
    #[arg(long)]
    host: String,
    #[arg(long)]
    port: u16,
}

fn succinct_tx(tx: &LedgerTxEvent<TxViewMut>) -> (TransactionHash, u64) {
    let (LedgerTxEvent::TxApplied { tx, block_number, .. }
    | LedgerTxEvent::TxUnapplied { tx, block_number, .. }) = tx;
    (tx.hash, *block_number)
}
