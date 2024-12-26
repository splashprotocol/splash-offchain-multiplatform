use clap::Parser;
use cml_chain::transaction::Transaction;
use cml_crypto::TransactionHash;
use either::Either;
use futures::channel::mpsc;
use futures::stream::FuturesUnordered;
use futures::{stream, stream_select, Stream, StreamExt};
use log::info;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing_subscriber::fmt::Subscriber;

use crate::config::AppConfig;
use crate::event::LpEvent;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::execution_part_stream;
use bloom_offchain::execution_engine::funding_effect::FundingEvent;
use bloom_offchain::execution_engine::liquidity_book::TLB;
use bloom_offchain::execution_engine::multi_pair::MultiPair;
use bloom_offchain::execution_engine::storage::InMemoryStateIndex;
use bloom_offchain_cardano::event_sink::context::{HandlerContext, HandlerContextProto};
use bloom_offchain_cardano::event_sink::entity_index::InMemoryEntityIndex;
use bloom_offchain_cardano::event_sink::handler::{
    FundingEventHandler, PairUpdateHandler, SpecializedHandler,
};
use bloom_offchain_cardano::event_sink::order_index::InMemoryKvIndex;
use bloom_offchain_cardano::event_sink::tx_view::TxViewMut;
use bloom_offchain_cardano::execution_engine::backlog::interpreter::SpecializedInterpreterViaRunOrder;
use bloom_offchain_cardano::execution_engine::interpreter::CardanoRecipeInterpreter;
use bloom_offchain_cardano::integrity::CheckIntegrity;
use bloom_offchain_cardano::orders::adhoc::AdhocFeeStructure;
use bloom_offchain_cardano::orders::AnyOrder;
use bloom_offchain_cardano::partitioning::select_partition;
use bloom_offchain_cardano::validation_rules::ValidationRules;
use cardano_chain_sync::cache::LedgerCacheRocksDB;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_chain_sync::event_source::ledger_transactions;
use cardano_explorer::Maestro;
use cardano_mempool_sync::client::LocalTxMonitorClient;
use cardano_mempool_sync::data::MempoolUpdate;
use cardano_mempool_sync::mempool_stream;
use spectrum_cardano_lib::constants::{CONWAY_ERA_ID, SAFE_BLOCK_TIME};
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::{constants, OutputRef, Token};
use spectrum_offchain::backlog::{BacklogCapacity, HotPriorityBacklog};
use spectrum_offchain::clock::SystemClock;
use spectrum_offchain::domain::event::{Channel, Transition};
use spectrum_offchain::domain::order::OrderUpdate;
use spectrum_offchain::domain::Baked;
use spectrum_offchain::event_sink::event_handler::{forward_to, EventHandler};
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain::partitioning::Partitioned;
use spectrum_offchain::reporting::{reporting_stream, ReportingAgent};
use spectrum_offchain::streaming::run_stream;
use spectrum_offchain::tracing::WithTracing;
use spectrum_offchain_cardano::collateral::pull_collateral;
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::data::pool::AnyPool;
use spectrum_offchain_cardano::deployment::{DeployedValidators, ProtocolDeployment, ProtocolScriptHashes};
use spectrum_offchain_cardano::prover::operator::OperatorProver;
use spectrum_offchain_cardano::tx_submission::{tx_submission_agent_stream, TxSubmissionAgent};
use spectrum_streaming::StreamExt as StreamExtAlt;

mod config;
mod event;
mod handler;
mod tx_view;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");
    let config_integrity_violations = config.check_integrity();
    if !config_integrity_violations.is_empty() {
        panic!("Malformed configuration: {}", config_integrity_violations);
    }

    let raw_deployment = std::fs::read_to_string(args.deployment_path).expect("Cannot load deployment file");
    let deployment: DeployedValidators =
        serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting Off-Chain Agent ..");

    let rollback_in_progress = Arc::new(AtomicBool::new(false));

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

    let (spec_upd_snd_p1, spec_upd_recv_p1) =
        mpsc::channel::<(PairId, LpEvent)>(config.event_feed_buffer_size);

    // todo: add handler
    let handlers_ledger: Vec<Box<dyn EventHandler<LedgerTxEvent<TxViewMut>> + Send>> = vec![];

    let (signal_tip_reached_snd, signal_tip_reached_recv) = broadcast::channel(1);

    let ledger_stream = Box::pin(ledger_transactions(
        chain_sync_cache,
        chain_sync_stream(chain_sync, signal_tip_reached_snd),
        config.chain_sync.disable_rollbacks_until,
        config.chain_sync.replay_from_point,
        rollback_in_progress,
    ))
    .await
    .map(|ev| match ev {
        LedgerTxEvent::TxApplied {
            tx,
            slot,
            block_number,
        } => LedgerTxEvent::TxApplied {
            tx: TxViewMut::from(tx),
            slot,
            block_number,
        },
        LedgerTxEvent::TxUnapplied {
            tx,
            slot,
            block_number,
        } => LedgerTxEvent::TxUnapplied {
            tx: TxViewMut::from(tx),
            slot,
            block_number,
        },
    });

    let process_ledger_events_stream = process_events(ledger_stream, handlers_ledger);

    let processes = FuturesUnordered::new();

    let process_ledger_events_stream_handle = tokio::spawn(run_stream(process_ledger_events_stream));
    processes.push(process_ledger_events_stream_handle);

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
}
