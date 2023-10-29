use std::sync::{Arc, Once};

use amm::event_handlers::ConfirmedUpdateHandler;
use clap::Parser;
use cml_multi_era::babbage::{BabbageBlock, BabbageTransaction};
use futures::channel::mpsc;
use futures::stream::select_all;
use futures::StreamExt;
use spectrum_offchain::box_resolver::process::pool_tracking_stream_simple;
use tokio::sync::Mutex;
use tracing_subscriber::fmt::Subscriber;

use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_chain_sync::event_source::event_source_ledger;
use spectrum_offchain::box_resolver::persistence::inmemory::InMemoryEntityRepo;
use spectrum_offchain::box_resolver::persistence::noop::NoopEntityRepo;
use spectrum_offchain::data::unique_entity::{EitherMod, StateUpdate};
use spectrum_offchain::event_sink::event_handler::{EventHandler, NoopDefaultHandler};
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain::streaming::boxed;

use crate::amm::pool::CFMMPool;
use crate::config::AppConfig;

mod amm;
mod config;

#[tokio::main]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    let chain_sync = ChainSyncClient::<BabbageBlock>::init(
        config.node.path,
        config.node.magic,
        config.chain_sync.starting_point,
    )
    .await
    .expect("ChainSync initialization failed");

    let signal_tip_reached: Once = Once::new();
    let ledger_stream = Box::pin(event_source_ledger(chain_sync_stream(
        chain_sync,
        Some(&signal_tip_reached),
    )));

    let (pools_snd, pools_recv) = mpsc::channel::<EitherMod<StateUpdate<CFMMPool>>>(128);
    // This technically disables pool lookups in TX.inputs.
    let noop_pool_repo = Arc::new(Mutex::new(NoopEntityRepo));
    let pools_handler_ledger =
        ConfirmedUpdateHandler::<_, CFMMPool, _>::new(pools_snd.clone(), Arc::clone(&noop_pool_repo));
    let pool_repo = Arc::new(Mutex::new(InMemoryEntityRepo::new()));
    let pool_tracking_stream = pool_tracking_stream_simple(pools_recv, pool_repo);

    let handlers_ledger: Vec<Box<dyn EventHandler<LedgerTxEvent<BabbageTransaction>>>> =
        vec![Box::new(pools_handler_ledger)];

    let default_handler = NoopDefaultHandler;

    let process_ledger_events_stream = process_events(ledger_stream, handlers_ledger, default_handler);

    let mut app = select_all(vec![
        boxed(process_ledger_events_stream),
        boxed(pool_tracking_stream),
    ]);

    loop {
        app.select_next_some().await;
    }
}

#[derive(Parser)]
#[command(name = "coinshark-cardano")]
#[command(author = "Spectrum Finance")]
#[command(version = "1.0.0")]
#[command(about = "Coinshark Bot", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
}
