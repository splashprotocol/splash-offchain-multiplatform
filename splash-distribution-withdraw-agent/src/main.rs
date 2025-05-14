use crate::config::withdraw_config::WithdrawConfig;
use crate::context::app_context::AppContext;
use crate::handlers::DAOEntitiesHandler;
use async_primitives::beacon::Beacon;
use bloom_offchain_cardano::event_sink::tx_view::TxViewMut;
use cardano_chain_sync::atomic_flow::atomic_block_flow;
use cardano_chain_sync::cache::LedgerCacheRocksDB;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_chain_sync::event_source::ledger_transactions;
use cardano_explorer::AnyExplorer;
use clap::Parser;
use cml_chain::transaction::Transaction;
use cml_crypto::TransactionHash;
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::{channel::mpsc, stream::FuturesUnordered, StreamExt};
use log::info;
use spectrum_cardano_lib::constants::CONWAY_ERA_ID;
use spectrum_cardano_lib::constants::SAFE_BLOCK_TIME;
use spectrum_offchain::event_sink::event_handler::{forward_with, EventHandler};
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain_cardano::persistent_index::IndexRocksDB;
use spectrum_offchain_cardano::prover::operator::OperatorProver;
use spectrum_offchain_cardano::tx_submission::{
    tx_submission_agent_stream, TxSubmissionAgent, TxSubmissionChannel,
};
use spectrum_offchain_cardano::tx_tracker::{new_tx_tracker_bundle, TxTrackerChannel};
use spectrum_streaming::{boxed, run_stream};
use splash_dao_offchain::handler::DaoHandler;
use splash_dao_offchain::state_projection::StateProjectionRocksDB;
use splash_distribution::buffered_wallet_holder::BufferedWalletsHolder;
use splash_distribution::db::distribution_db::DistributionDb;
use splash_distribution::distributor::smart_farm_withdrawer::SmartFarmWithdrawer;
use splash_distribution::distributor::user_requests_processor::UserRequestsProcessor;
use splash_distribution::entities::events::smart_farms::withdraw::SmartFarmWithdraw;
use splash_distribution::http_clients::validator_client::HttpClient;
use splash_lp_indexer::position_db::PositionDB;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub mod config;
mod context;
mod handlers;
mod onchain;
mod pipeline;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: WithdrawConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    let chain_sync_cache = Arc::new(Mutex::new(LedgerCacheRocksDB::new(
        config.clone().chain_sync.db_path,
    )));

    let chain_sync = ChainSyncClient::init(
        Arc::clone(&chain_sync_cache),
        config.node.path.clone(),
        config.node.magic,
        config.chain_sync.starting_point,
    )
    .await
    .expect("ChainSync initialization failed");

    let operator_prover = OperatorProver::new(config.clone().distributor_key);

    let distribution_db = Arc::new(Mutex::new(DistributionDb::new(config.clone().db_path.clone())));

    let buffered_wallets_holder = Arc::new(Mutex::new(BufferedWalletsHolder::new(
        config.buffered_wallets_db.clone(),
    )));

    let buffered_wallets_storage = Arc::new(Mutex::new(DistributionDb::new(
        config.clone().buffered_wallets_storage,
    )));

    let smart_farm_storage = Arc::new(Mutex::new(DistributionDb::new(config.clone().smart_farm_storage)));

    let user_withdraw_storage = Arc::new(Mutex::new(DistributionDb::new(
        config.clone().users_withdraw_storage,
    )));

    let perm_manager_storage = Arc::new(Mutex::new(DistributionDb::new(
        config.clone().perm_manager_storage,
    )));

    let (sf_withdraw_requests_sender, sf_withdraw_requests_receiver) =
        // todo: 100 in config
        tokio::sync::mpsc::channel::<SmartFarmWithdraw>(100);

    let mk_path = |name: &str| {
        if name.ends_with('/') {
            format!("{}{}", config.persistence_stores_root_dir, name)
        } else {
            format!("{}/{}", config.persistence_stores_root_dir, name)
        }
    };

    let withdrawer = SmartFarmWithdrawer::new(
        sf_withdraw_requests_receiver,
        buffered_wallets_holder.clone(),
        distribution_db.clone(),
        StateProjectionRocksDB::new(mk_path("perm_manager")),
    );

    let http_client = HttpClient::new();

    let mut requests_processor: UserRequestsProcessor<DistributionDb, HttpClient, HttpClient> =
        UserRequestsProcessor::new(
            distribution_db.clone(),
            http_client.clone(),
            buffered_wallets_holder,
            http_client,
            sf_withdraw_requests_sender,
        );

    let (failed_txs_snd, failed_txs_recv) =
        tokio::sync::mpsc::channel::<Transaction>(config.tx_submission_buffer_size);
    let (confirmed_txs_snd, confirmed_txs_recv) =
        mpsc::channel::<(TransactionHash, u64)>(config.tx_submission_buffer_size);
    let max_confirmation_delay_blocks = config.event_cache_ttl.as_secs() / SAFE_BLOCK_TIME.as_secs();
    info!("max_confirmation_delay_blocks: {}", max_confirmation_delay_blocks);

    let explorer = AnyExplorer::new(&config.clone().explorer, config.clone().network_id)
        .await
        .expect("Explorer initialization failed");

    let ctx: AppContext = AppContext::from_config(config.clone(), &explorer).await;

    let state_synced = Beacon::relaxed(false);

    let processes: FuturesUnordered<JoinHandle<()>> = FuturesUnordered::new();

    let rollback_in_progress = Beacon::strong(false);

    let ledger_stream = Box::pin(ledger_transactions(
        chain_sync_cache,
        chain_sync_stream(chain_sync, state_synced.clone()),
        config.chain_sync.disable_rollbacks_until,
        config.chain_sync.replay_from_point,
        rollback_in_progress,
    ))
    .await
    .map(|ev: LedgerTxEvent<Either<BabbageTransaction, Transaction>>| ev.map(TxViewMut::from));

    //let (ledger_event_snd, ledger_event_rcv) = tokio::sync::mpsc::channel(100);

    let handlers: Vec<Box<dyn EventHandler<LedgerTxEvent<TxViewMut>> + Send>> = vec![
        Box::new(DAOEntitiesHandler::new(
            smart_farm_storage,
            user_withdraw_storage,
            buffered_wallets_storage,
            perm_manager_storage,
            ctx.clone(),
        )),
        Box::new(forward_with(confirmed_txs_snd, succinct_tx)),
    ];

    let process_ledger_events_stream = process_events(ledger_stream, handlers);

    let process_ledger_events_stream_handle = tokio::spawn(run_stream(process_ledger_events_stream));
    processes.push(process_ledger_events_stream_handle);

    let (tx_tracker_agent, tx_tracker_channel) = new_tx_tracker_bundle(
        confirmed_txs_recv,
        tokio_util::sync::PollSender::new(failed_txs_snd), // Need to wrap the Sender to satisfy Sink trait
        config.tx_submission_buffer_size,
        max_confirmation_delay_blocks,
    );
    let (tx_submission_agent, tx_submission_channel) =
        TxSubmissionAgent::<CONWAY_ERA_ID, Transaction, _>::new(
            tx_tracker_channel,
            config.node,
            config.tx_submission_buffer_size,
        )
        .await
        .unwrap();

    let withdraw_stream =
        withdrawer.withdrawer_stream(ctx.clone(), operator_prover, tx_submission_channel.clone());

    let withdrawer_stream_to_add = tokio::spawn(run_stream(withdraw_stream));

    processes.push(withdrawer_stream_to_add);

    let requests_processor_stream = tokio::spawn({
        run_stream(
            requests_processor.user_requests_processor_stream(ctx.clone(), tx_submission_channel.clone()),
        )
    });

    processes.push(requests_processor_stream);

    let tx_submission_stream = tokio::spawn(run_stream(tx_submission_agent_stream(tx_submission_agent)));

    processes.push(tx_submission_stream);

    let tx_tracker_handle = tokio::spawn(tx_tracker_agent.run());
    processes.push(tx_tracker_handle);

    run_stream(processes).await;
}

fn succinct_tx(tx: LedgerTxEvent<TxViewMut>) -> (TransactionHash, u64) {
    let (LedgerTxEvent::TxApplied { tx, block_number, .. }
    | LedgerTxEvent::TxUnapplied { tx, block_number, .. }) = tx;
    (tx.hash, block_number)
}

#[derive(Parser)]
#[command(name = "splash-withdraw-agent")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Splash Withdraw Agent", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
}
