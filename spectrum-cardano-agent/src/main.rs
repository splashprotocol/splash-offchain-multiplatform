use std::sync::{Arc, Once};

use clap::Parser;
use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::genesis::network_info::NetworkInfo;
use cml_chain::transaction::Transaction;
use cml_crypto::PrivateKey;
use cml_multi_era::babbage::BabbageTransaction;
use futures::channel::mpsc;
use futures::stream::select_all;
use futures::{Stream, StreamExt};
use log::info;
use tokio::sync::Mutex;
use tracing_subscriber::fmt::Subscriber;

use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_chain_sync::event_source::ledger_transactions;
use cardano_explorer::client::Explorer;
use cardano_mempool_sync::client::LocalTxMonitorClient;
use cardano_mempool_sync::data::MempoolUpdate;
use cardano_mempool_sync::mempool_stream;
use cardano_submit_api::client::LocalTxSubmissionClient;
use spectrum_cardano_lib::constants::BABBAGE_ERA_ID;
use spectrum_offchain::backlog::process::hot_backlog_stream;
use spectrum_offchain::backlog::HotPriorityBacklog;
use spectrum_offchain::box_resolver::persistence::inmemory::InMemoryEntityRepo;
use spectrum_offchain::box_resolver::persistence::noop::NoopEntityRepo;
use spectrum_offchain::box_resolver::process::pool_tracking_stream;
use spectrum_offchain::data::order::{OrderLink, OrderUpdate};
use spectrum_offchain::data::unique_entity::{EitherMod, StateUpdate};
use spectrum_offchain::event_sink::event_handler::{EventHandler, NoopDefaultHandler};
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain::executor::{executor_stream, HotOrderExecutor};
use spectrum_offchain::network::Network;
use spectrum_offchain::partitioning::Partitioned;
use spectrum_offchain::streaming::boxed;
use spectrum_offchain_cardano::collaterals::{Collaterals, CollateralsViaExplorer};
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::data::execution_context::ExecutionContext;
use spectrum_offchain_cardano::data::order::ClassicalOnChainOrder;
use spectrum_offchain_cardano::data::pool::CFMMPool;
use spectrum_offchain_cardano::data::ref_scripts::ReferenceOutputs;
use spectrum_offchain_cardano::data::{OnChain, PoolId};
use spectrum_offchain_cardano::event_sink::handlers::reproducible::{
    ConfirmedUpdateHandler, UnconfirmedUpdateHandler,
};
use spectrum_offchain_cardano::event_sink::handlers::short_term::registry::EphemeralHotOrderRegistry;
use spectrum_offchain_cardano::event_sink::handlers::short_term::ClassicalOrderUpdatesHandler;
use spectrum_offchain_cardano::prover::operator::OperatorProver;
use spectrum_offchain_cardano::tx_submission::{tx_submission_agent_stream, TxRejected, TxSubmissionAgent};

use crate::config::AppConfig;

mod config;

#[tokio::main]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting offchain service ..");

    let explorer = Explorer::new(config.explorer);

    let ref_scripts = ReferenceOutputs::pull(config.ref_scripts, explorer)
        .await
        .expect("Ref scripts initialization failed");

    let chain_sync = ChainSyncClient::init(
        config.node.path,
        config.node.magic,
        config.chain_sync.starting_point,
    )
    .await
    .expect("ChainSync initialization failed");

    // n2c clients:
    let mempool_sync = LocalTxMonitorClient::connect(config.node.path, config.node.magic)
        .await
        .expect("MempoolSync initialization failed");
    let tx_submission_client =
        LocalTxSubmissionClient::<BABBAGE_ERA_ID, Transaction>::init(config.node.path, config.node.magic)
            .await
            .expect("LocalTxSubmission initialization failed");
    let (tx_submission_agent, tx_submission_channel) =
        TxSubmissionAgent::new(tx_submission_client, config.tx_submission_buffer_size);

    // prepare upstreams
    let tx_submission_stream = tx_submission_agent_stream(tx_submission_agent);
    let signal_tip_reached = Once::new();
    let ledger_stream = Box::pin(ledger_transactions(chain_sync_stream(
        chain_sync,
        Some(&signal_tip_reached),
    )));
    let mempool_stream = mempool_stream(&mempool_sync, Some(&signal_tip_reached));

    let (operator_sk, operator_pkh, operator_addr) =
        operator_creds(config.batcher_private_key, NetworkInfo::mainnet());

    let explorer_based_requestor = CollateralsViaExplorer::new(operator_pkh.to_hex(), explorer);

    let collateral = explorer_based_requestor
        .get_collateral()
        .await
        .expect("Couldn't retrieve collateral");

    let ctx = ExecutionContext::new(operator_addr, ref_scripts, collateral);

    let p1 = new_partition(
        tx_submission_channel.clone(),
        &operator_sk,
        Some(&signal_tip_reached),
        ctx.clone(),
    );
    let p2 = new_partition(
        tx_submission_channel.clone(),
        &operator_sk,
        Some(&signal_tip_reached),
        ctx.clone(),
    );
    let p3 = new_partition(
        tx_submission_channel.clone(),
        &operator_sk,
        Some(&signal_tip_reached),
        ctx.clone(),
    );
    let p4 = new_partition(
        tx_submission_channel,
        &operator_sk,
        Some(&signal_tip_reached),
        ctx,
    );

    let partitioned_backlog = Partitioned::<NUM_PARTITIONS, PoolId, _>::new([
        Arc::clone(&p1.backlog),
        Arc::clone(&p2.backlog),
        Arc::clone(&p3.backlog),
        Arc::clone(&p4.backlog),
    ]);
    let (orders_snd, orders_recv) =
        mpsc::channel::<OrderUpdate<ClassicalOnChainOrder, OrderLink<ClassicalOnChainOrder>>>(128);
    let orders_registry = Arc::new(Mutex::new(
        EphemeralHotOrderRegistry::<ClassicalOnChainOrder>::new(),
    ));
    let orders_handler_ledger = ClassicalOrderUpdatesHandler::<_, ClassicalOnChainOrder, _>::new(
        orders_snd.clone(),
        orders_registry.clone(),
    );
    let orders_handler_mempool =
        ClassicalOrderUpdatesHandler::<_, ClassicalOnChainOrder, _>::new(orders_snd, orders_registry);
    let backlog_stream = hot_backlog_stream(partitioned_backlog, orders_recv);

    let (pools_snd, pools_recv) = mpsc::channel::<EitherMod<StateUpdate<OnChain<CFMMPool>>>>(128);
    // This technically disables pool lookups in TX.inputs.
    let noop_pool_repo = Arc::new(Mutex::new(NoopEntityRepo));
    let pools_handler_ledger = ConfirmedUpdateHandler::<_, OnChain<CFMMPool>, _>::new(
        pools_snd.clone(),
        Arc::clone(&noop_pool_repo),
    );
    let pools_handler_mempool =
        UnconfirmedUpdateHandler::<_, OnChain<CFMMPool>, _>::new(pools_snd, noop_pool_repo);
    let partitioned_pool_repo = Partitioned::<NUM_PARTITIONS, PoolId, _>::new([
        Arc::clone(&p1.pool_repo),
        Arc::clone(&p2.pool_repo),
        Arc::clone(&p3.pool_repo),
        Arc::clone(&p4.pool_repo),
    ]);
    let pool_tracking_stream = pool_tracking_stream(pools_recv, partitioned_pool_repo);

    let handlers_ledger: Vec<Box<dyn EventHandler<LedgerTxEvent<BabbageTransaction>>>> =
        vec![Box::new(pools_handler_ledger), Box::new(orders_handler_ledger)];

    let handlers_mempool: Vec<Box<dyn EventHandler<MempoolUpdate<BabbageTransaction>>>> =
        vec![Box::new(pools_handler_mempool), Box::new(orders_handler_mempool)];

    let process_ledger_events_stream = process_events(ledger_stream, handlers_ledger);
    let process_mempool_events_stream = process_events(mempool_stream, handlers_mempool);

    let mut app = select_all(vec![
        boxed(process_ledger_events_stream),
        boxed(process_mempool_events_stream),
        boxed(backlog_stream),
        boxed(pool_tracking_stream),
        boxed(tx_submission_stream),
        boxed(p1.executor_stream),
        boxed(p2.executor_stream),
        boxed(p3.executor_stream),
        boxed(p4.executor_stream),
    ]);

    loop {
        app.select_next_some().await;
    }
}

const NUM_PARTITIONS: usize = 4;

struct ExecPartition<S, Backlog, Pools> {
    executor_stream: S,
    backlog: Arc<Mutex<Backlog>>,
    pool_repo: Arc<Mutex<Pools>>,
}

fn new_partition<'a, Net>(
    network: Net,
    operator_sk: &'a PrivateKey,
    signal_tip_reached: Option<&'a Once>,
    ctx: ExecutionContext,
) -> ExecPartition<
    impl Stream<Item = ()> + 'a,
    HotPriorityBacklog<ClassicalOnChainOrder>,
    InMemoryEntityRepo<OnChain<CFMMPool>>,
>
where
    Net: Network<Transaction, TxRejected> + 'a,
{
    let backlog = Arc::new(Mutex::new(HotPriorityBacklog::new(43)));
    let pool_repo = Arc::new(Mutex::new(InMemoryEntityRepo::new()));
    let prover = OperatorProver::new(operator_sk);
    let executor: HotOrderExecutor<
        _,
        _,
        _,
        _,
        _,
        ClassicalOnChainOrder,
        OnChain<CFMMPool>,
        SignedTxBuilder,
        Transaction,
        TxRejected,
    > = HotOrderExecutor::new(network, Arc::clone(&backlog), Arc::clone(&pool_repo), prover, ctx);
    let executor_stream = executor_stream(executor, signal_tip_reached);
    ExecPartition {
        executor_stream,
        backlog,
        pool_repo,
    }
}

#[derive(Parser)]
#[command(name = "spectrum-cardano-agent")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Spectrum DEX Off-Chain Agent", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
}
