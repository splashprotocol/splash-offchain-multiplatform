use std::sync::{Arc, Once};

use clap::Parser;
use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::genesis::network_info::NetworkInfo;
use cml_chain::transaction::Transaction;
use futures::channel::mpsc;
use futures::stream::select_all;
use futures::{Stream, StreamExt};
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing_subscriber::fmt::Subscriber;

use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::{ChainSyncClient, ChainSyncConf};
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_chain_sync::event_source::event_source_ledger;
use cardano_explorer::client::Explorer;
use cardano_explorer::data::ExplorerConfig;
use cardano_submit_api::client::{LocalTxSubmissionClient, LocalTxSubmissionConf};
use spectrum_cardano_lib::constants::BABBAGE_ERA_ID;
use spectrum_offchain::backlog::process::hot_backlog_stream;
use spectrum_offchain::backlog::HotPriorityBacklog;
use spectrum_offchain::box_resolver::persistence::inmemory::InMemoryEntityRepo;
use spectrum_offchain::box_resolver::persistence::noop::NoopEntityRepo;
use spectrum_offchain::box_resolver::process::pool_tracking_stream;
use spectrum_offchain::data::order::{OrderLink, OrderUpdate};
use spectrum_offchain::data::unique_entity::{Confirmed, StateUpdate};
use spectrum_offchain::event_sink::event_handler::{EventHandler, NoopDefaultHandler};
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain::executor::{executor_stream, HotOrderExecutor};
use spectrum_offchain::network::Network;
use spectrum_offchain::partitioning::Partitioned;
use spectrum_offchain::streaming::boxed;

use crate::collateral_storage::CollateralStorage;
use crate::creds::operator_creds;
use crate::data::execution_context::ExecutionContext;
use crate::data::order::ClassicalOnChainOrder;
use crate::data::pool::CFMMPool;
use crate::data::ref_scripts::RefScriptsOutputs;
use crate::data::{OnChain, PoolId};
use crate::event_sink::handlers::order::registry::EphemeralHotOrderRegistry;
use crate::event_sink::handlers::order::ClassicalOrderUpdatesHandler;
use crate::event_sink::handlers::pool::ConfirmedUpdateHandler;
use crate::prover::noop::NoopProver;
use crate::tx_submission::{tx_submission_agent_stream, TxRejected, TxSubmissionAgent};

mod cardano;
mod collateral_storage;
mod constants;
mod creds;
mod data;
mod event_sink;
mod prover;
mod tx_submission;

#[tokio::main]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file"); //use crate for config files

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    let explorer = Explorer::new(config.explorer_config);

    let ref_scripts = RefScriptsOutputs::new(config.ref_scripts, explorer)
        .await
        .expect("Ref scripts initialization failed");

    let chain_sync = ChainSyncClient::init(config.chain_sync)
        .await
        .expect("ChainSync initialization failed");
    let tx_submission_client = LocalTxSubmissionClient::<BABBAGE_ERA_ID>::init(config.local_tx_submission)
        .await
        .expect("LocalTxSubmission initialization failed");
    let (tx_submission_agent, tx_submission_channel) =
        TxSubmissionAgent::new(tx_submission_client, config.tx_submission_buffer_size);
    let tx_submission_stream = tx_submission_agent_stream(tx_submission_agent);
    let signal_tip_reached: Once = Once::new();
    let ledger_stream = Box::pin(event_source_ledger(chain_sync_stream(
        chain_sync,
        Some(&signal_tip_reached),
    )));

    let (operator_sk, operator_pkh, operator_addr) =
        operator_creds(config.batcher_private_key, NetworkInfo::mainnet());

    let collateral_storage = CollateralStorage::new(operator_pkh.to_hex());

    let collateral = collateral_storage
        .get_collateral(explorer)
        .await
        .expect("Couldn't retrieve collateral");

    let ctx = ExecutionContext::new(operator_addr, &operator_sk, ref_scripts, collateral);

    let p1 = new_partition(tx_submission_channel.clone(), Some(&signal_tip_reached), ctx.clone());
    let p2 = new_partition(tx_submission_channel.clone(), Some(&signal_tip_reached), ctx.clone());
    let p3 = new_partition(tx_submission_channel.clone(), Some(&signal_tip_reached), ctx.clone());
    let p4 = new_partition(tx_submission_channel, Some(&signal_tip_reached), ctx);

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
    let orders_handler =
        ClassicalOrderUpdatesHandler::<_, ClassicalOnChainOrder, _>::new(orders_snd, orders_registry);
    let backlog_stream = hot_backlog_stream(partitioned_backlog, orders_recv);

    let (pools_snd, pools_recv) = mpsc::channel::<Confirmed<StateUpdate<OnChain<CFMMPool>>>>(128);
    // This technically disables pool lookups in TX.inputs.
    let noop_pool_repo = Arc::new(Mutex::new(NoopEntityRepo));
    let pools_handler = ConfirmedUpdateHandler::<_, OnChain<CFMMPool>, _>::new(pools_snd, noop_pool_repo);
    let partitioned_pool_repo = Partitioned::<NUM_PARTITIONS, PoolId, _>::new([
        Arc::clone(&p1.pool_repo),
        Arc::clone(&p2.pool_repo),
        Arc::clone(&p3.pool_repo),
        Arc::clone(&p4.pool_repo),
    ]);
    let pool_tracking_stream = pool_tracking_stream(pools_recv, partitioned_pool_repo);

    let handlers: Vec<Box<dyn EventHandler<LedgerTxEvent>>> =
        vec![Box::new(pools_handler), Box::new(orders_handler)];

    let default_handler = NoopDefaultHandler;
    let process_events_stream = process_events(ledger_stream, handlers, default_handler);

    let mut app = select_all(vec![
        boxed(process_events_stream),
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
    signal_tip_reached: Option<&'a Once>,
    ctx: ExecutionContext<'a>,
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
    let prover = NoopProver {};
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefScriptsConfig {
    pool_v1_ref: String,
    pool_v2_ref: String,
    swap_ref: String,
    deposit_ref: String,
    redeem_ref: String,
}

#[derive(Deserialize)]
#[serde(bound = "'de: 'a")]
#[serde(rename_all = "camelCase")]
struct AppConfig<'a> {
    chain_sync: ChainSyncConf<'a>,
    local_tx_submission: LocalTxSubmissionConf<'a>,
    tx_submission_buffer_size: usize,
    batcher_private_key: &'a str, //todo: to cypher container
    ref_scripts: RefScriptsConfig,
    explorer_config: ExplorerConfig<'a>,
}

#[derive(Parser)]
#[command(name = "spectrum-offchain-cardano")]
#[command(author = "Spectrum Finance")]
#[command(version = "1.0.0")]
#[command(about = "Spectrum DEX Offchain Bot", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
}
