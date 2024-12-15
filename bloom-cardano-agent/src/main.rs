use clap::Parser;
use cml_chain::transaction::Transaction;
use cml_crypto::TransactionHash;
use either::Either;
use futures::channel::mpsc;
use futures::stream::FuturesUnordered;
use futures::{stream_select, Stream, StreamExt};
use log::info;
use std::future;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing_subscriber::fmt::Subscriber;

use crate::config::AppConfig;
use crate::context::{ExecutionContext, MakerContext};
use crate::entity::{AtomicCardanoEntity, EvolvingCardanoEntity};
use async_primitives::beacon::Beacon;
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
use spectrum_offchain::tracing::WithTracing;
use spectrum_offchain_cardano::collateral::pull_collateral;
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::data::dao_request::DAOContext;
use spectrum_offchain_cardano::data::order::Order;
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::data::pool::AnyPool;
use spectrum_offchain_cardano::deployment::{DeployedValidators, ProtocolDeployment, ProtocolScriptHashes};
use spectrum_offchain_cardano::prover::operator::OperatorProver;
use spectrum_offchain_cardano::tx_submission::{tx_submission_agent_stream, TxSubmissionAgent};
use spectrum_offchain_cardano::tx_tracker::{new_tx_tracker_bundle, TxTrackerChannel};
use spectrum_streaming::{run_stream, StreamExt as StreamExtAlt};

mod config;
mod context;
mod entity;

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

    let raw_validation_rules =
        std::fs::read_to_string(args.validation_rules_path).expect("Cannot load bounds file");
    let validation_rules: ValidationRules =
        serde_json::from_str(&raw_validation_rules).expect("Invalid bounds file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting Off-Chain Agent ..");

    let state_synced = Beacon::relaxed(false);
    let rollback_in_progress = Beacon::strong(false);

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

    // n2c clients:
    let mempool_sync =
        LocalTxMonitorClient::<Transaction>::connect(config.node.path.clone(), config.node.magic)
            .await
            .expect("MempoolSync initialization failed");

    let (failed_txs_snd, failed_txs_recv) = mpsc::channel(config.tx_submission_buffer_size);
    let (confirmed_txs_snd, confirmed_txs_recv) = mpsc::channel(config.tx_submission_buffer_size);
    let max_confirmation_delay_blocks = config.event_cache_ttl.as_secs() / SAFE_BLOCK_TIME.as_secs();
    let (tx_tracker_agent, tx_tracker_channel) = new_tx_tracker_bundle(
        confirmed_txs_recv,
        failed_txs_snd,
        config.tx_submission_buffer_size,
        max_confirmation_delay_blocks,
    );

    let (tx_submission_agent, tx_submission_channel) =
        TxSubmissionAgent::<CONWAY_ERA_ID, Transaction, TxTrackerChannel<TransactionHash, Transaction>>::new(
            tx_tracker_channel.clone(),
            config.node.clone(),
            config.tx_submission_buffer_size,
        )
        .await
        .expect("LocalTxSubmission initialization failed");
    let tx_submission_stream = tx_submission_agent_stream(tx_submission_agent);

    let (reporting_agent, reporting_channel) =
        ReportingAgent::new(config.reporting_endpoint, config.tx_submission_buffer_size).await;
    let reporting_stream = reporting_stream(reporting_agent);

    let (operator_paycred, collateral_address, funding_addresses) =
        operator_creds(config.operator_key.as_str(), config.network_id);

    info!(
        "Expecting collateral at {}",
        collateral_address.clone().address().to_bech32(None).unwrap()
    );

    let collateral = pull_collateral(collateral_address, &explorer)
        .await
        .expect("Couldn't retrieve collateral");

    let (pair_upd_snd_p1, pair_upd_recv_p1) =
        mpsc::channel::<(PairId, Channel<Transition<EvolvingCardanoEntity>>)>(config.event_feed_buffer_size);
    let (pair_upd_snd_p2, pair_upd_recv_p2) =
        mpsc::channel::<(PairId, Channel<Transition<EvolvingCardanoEntity>>)>(config.event_feed_buffer_size);
    let (pair_upd_snd_p3, pair_upd_recv_p3) =
        mpsc::channel::<(PairId, Channel<Transition<EvolvingCardanoEntity>>)>(config.event_feed_buffer_size);
    let (pair_upd_snd_p4, pair_upd_recv_p4) =
        mpsc::channel::<(PairId, Channel<Transition<EvolvingCardanoEntity>>)>(config.event_feed_buffer_size);

    let partitioned_pair_upd_snd =
        Partitioned::new([pair_upd_snd_p1, pair_upd_snd_p2, pair_upd_snd_p3, pair_upd_snd_p4]);

    let (spec_upd_snd_p1, spec_upd_recv_p1) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.event_feed_buffer_size);
    let (spec_upd_snd_p2, spec_upd_recv_p2) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.event_feed_buffer_size);
    let (spec_upd_snd_p3, spec_upd_recv_p3) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.event_feed_buffer_size);
    let (spec_upd_snd_p4, spec_upd_recv_p4) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.event_feed_buffer_size);

    let partitioned_spec_upd_snd =
        Partitioned::new([spec_upd_snd_p1, spec_upd_snd_p2, spec_upd_snd_p3, spec_upd_snd_p4]);

    let (funding_upd_snd_p1, funding_upd_recv_p1) =
        mpsc::channel::<FundingEvent<FinalizedTxOut>>(config.event_feed_buffer_size);
    let (funding_upd_snd_p2, funding_upd_recv_p2) =
        mpsc::channel::<FundingEvent<FinalizedTxOut>>(config.event_feed_buffer_size);
    let (funding_upd_snd_p3, funding_upd_recv_p3) =
        mpsc::channel::<FundingEvent<FinalizedTxOut>>(config.event_feed_buffer_size);
    let (funding_upd_snd_p4, funding_upd_recv_p4) =
        mpsc::channel::<FundingEvent<FinalizedTxOut>>(config.event_feed_buffer_size);

    let partitioned_funding_event_snd = Partitioned::new([
        funding_upd_snd_p1,
        funding_upd_snd_p2,
        funding_upd_snd_p3,
        funding_upd_snd_p4,
    ]);

    let entity_index = Arc::new(Mutex::new(InMemoryEntityIndex::new(config.event_cache_ttl)));
    let spec_order_index = Arc::new(Mutex::new(
        InMemoryKvIndex::new(config.event_cache_ttl, SystemClock).with_tracing("spec_order_index"),
    ));
    let funding_index = Arc::new(Mutex::new(
        InMemoryKvIndex::new(config.event_cache_ttl, SystemClock).with_tracing("funding_index"),
    ));
    let dao_ctx: DAOContext = config.dao_config.clone().into();
    let handler_context = HandlerContextProto {
        executor_cred: operator_paycred,
        scripts: ProtocolScriptHashes::from(&protocol_deployment),
        adhoc_fee_structure: AdhocFeeStructure::empty(),
        validation_rules,
        dao_context: dao_ctx,
    };
    let general_upd_handler: PairUpdateHandler<4, _, _, _, _, HandlerContextProto, HandlerContext<Token>> =
        PairUpdateHandler::new(
            partitioned_pair_upd_snd,
            Arc::clone(&entity_index),
            handler_context,
        );
    let spec_upd_handler = SpecializedHandler::<_, _, _, Token, HandlerContext<Token>>::new(
        PairUpdateHandler::new(partitioned_spec_upd_snd, entity_index, handler_context),
        spec_order_index,
    );
    let funding_event_handler = FundingEventHandler::new(
        partitioned_funding_event_snd,
        funding_addresses.clone(),
        collateral.reference(), // collateral cannot be used for funding.
        funding_index,
    );

    info!("Derived funding addresses: {}", funding_addresses);

    let handlers_ledger: Vec<Box<dyn EventHandler<LedgerTxEvent<TxViewMut>> + Send>> = vec![
        Box::new(general_upd_handler.clone()),
        Box::new(spec_upd_handler.clone()),
        Box::new(funding_event_handler.clone()),
        Box::new(forward_to(confirmed_txs_snd, succinct_tx)),
    ];

    let handlers_mempool: Vec<Box<dyn EventHandler<MempoolUpdate<TxViewMut>> + Send>> = vec![
        Box::new(general_upd_handler),
        Box::new(spec_upd_handler),
        Box::new(funding_event_handler),
    ];

    let prover = OperatorProver::new(config.operator_key);
    let recipe_interpreter = CardanoRecipeInterpreter::new(config.take_residual_fee);
    let spec_interpreter = SpecializedInterpreterViaRunOrder;
    let maker_context = MakerContext {
        time: 0.into(),
        execution_conf: config
            .execution
            .into_lb_config(validation_rules.limit_order.min_cost_per_ex_step.into()),
        backlog_capacity: BacklogCapacity::from(config.backlog_capacity),
    };
    let context_p1 = ExecutionContext {
        time: 0.into(),
        deployment: protocol_deployment.clone(),
        reward_addr: funding_addresses[0].clone().into(),
        backlog_capacity: BacklogCapacity::from(config.backlog_capacity),
        collateral: collateral.clone(),
        network_id: config.network_id,
        operator_cred: operator_paycred,
        dao_ctx,
        royalty_context: config.royalty_withdraw,
    };
    let context_p2 = ExecutionContext {
        time: 0.into(),
        deployment: protocol_deployment.clone(),
        reward_addr: funding_addresses[1].clone().into(),
        backlog_capacity: BacklogCapacity::from(config.backlog_capacity),
        collateral: collateral.clone(),
        network_id: config.network_id,
        operator_cred: operator_paycred,
        dao_ctx,
        royalty_context: config.royalty_withdraw,
    };
    let context_p3 = ExecutionContext {
        time: 0.into(),
        deployment: protocol_deployment.clone(),
        reward_addr: funding_addresses[2].clone().into(),
        backlog_capacity: BacklogCapacity::from(config.backlog_capacity),
        collateral: collateral.clone(),
        network_id: config.network_id,
        operator_cred: operator_paycred,
        dao_ctx,
        royalty_context: config.royalty_withdraw,
    };
    let context_p4 = ExecutionContext {
        time: 0.into(),
        deployment: protocol_deployment,
        reward_addr: funding_addresses[3].clone().into(),
        backlog_capacity: BacklogCapacity::from(config.backlog_capacity),
        collateral,
        network_id: config.network_id,
        operator_cred: operator_paycred,
        dao_ctx,
        royalty_context: config.royalty_withdraw,
    };

    let multi_book = MultiPair::new::<TLB<AnyOrder, AnyPool, PairId, ExUnits>>(maker_context.clone(), "Book");
    let multi_backlog = MultiPair::new::<WithTracing<HotPriorityBacklog<Bundled<Order, FinalizedTxOut>>>>(
        maker_context,
        "Backlog",
    );
    let state_index = InMemoryStateIndex::with_tracing();

    let execution_stream_p1 = execution_part_stream(
        state_index.clone(),
        multi_book.clone(),
        multi_backlog.clone(),
        context_p1,
        recipe_interpreter,
        spec_interpreter,
        prover.clone(),
        select_partition(
            merge_upstreams(
                pair_upd_recv_p1.buffered_within(config.event_feed_buffering_duration),
                spec_upd_recv_p1,
            ),
            config.partitioning.clone(),
        ),
        funding_upd_recv_p1,
        tx_submission_channel.clone(),
        reporting_channel.clone(),
        state_synced.clone(),
        rollback_in_progress.clone(),
    );
    let execution_stream_p2 = execution_part_stream(
        state_index.clone(),
        multi_book.clone(),
        multi_backlog.clone(),
        context_p2,
        recipe_interpreter,
        spec_interpreter,
        prover.clone(),
        select_partition(
            merge_upstreams(
                pair_upd_recv_p2.buffered_within(config.event_feed_buffering_duration),
                spec_upd_recv_p2,
            ),
            config.partitioning.clone(),
        ),
        funding_upd_recv_p2,
        tx_submission_channel.clone(),
        reporting_channel.clone(),
        state_synced.clone(),
        rollback_in_progress.clone(),
    );
    let execution_stream_p3 = execution_part_stream(
        state_index.clone(),
        multi_book.clone(),
        multi_backlog.clone(),
        context_p3,
        recipe_interpreter,
        spec_interpreter,
        prover.clone(),
        select_partition(
            merge_upstreams(
                pair_upd_recv_p3.buffered_within(config.event_feed_buffering_duration),
                spec_upd_recv_p3,
            ),
            config.partitioning.clone(),
        ),
        funding_upd_recv_p3,
        tx_submission_channel.clone(),
        reporting_channel.clone(),
        state_synced.clone(),
        rollback_in_progress.clone(),
    );
    let execution_stream_p4 = execution_part_stream(
        state_index,
        multi_book,
        multi_backlog,
        context_p4,
        recipe_interpreter,
        spec_interpreter,
        prover,
        select_partition(
            merge_upstreams(
                pair_upd_recv_p4.buffered_within(config.event_feed_buffering_duration),
                spec_upd_recv_p4,
            ),
            config.partitioning,
        ),
        funding_upd_recv_p4,
        tx_submission_channel,
        reporting_channel,
        state_synced.clone(),
        rollback_in_progress.clone(),
    );

    let ledger_stream = Box::pin(ledger_transactions(
        chain_sync_cache,
        chain_sync_stream(chain_sync, state_synced.clone()),
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

    let mempool_stream = mempool_stream(mempool_sync, tx_tracker_channel, failed_txs_recv, state_synced)
        .map(|ev| ev.map(TxViewMut::from));

    let process_ledger_events_stream = process_events(ledger_stream, handlers_ledger);
    let process_mempool_events_stream = process_events(mempool_stream, handlers_mempool);

    let processes = FuturesUnordered::new();

    let process_ledger_events_stream_handle = tokio::spawn(run_stream(process_ledger_events_stream));
    processes.push(process_ledger_events_stream_handle);

    let process_mempool_events_stream_handle = if !config.disable_mempool {
        tokio::spawn(run_stream(process_mempool_events_stream))
    } else {
        tokio::spawn(future::ready(()))
    };
    processes.push(process_mempool_events_stream_handle);

    let execution_stream_p1_handle = tokio::spawn(run_stream(execution_stream_p1));
    processes.push(execution_stream_p1_handle);

    let execution_stream_p2_handle = tokio::spawn(run_stream(execution_stream_p2));
    processes.push(execution_stream_p2_handle);

    let execution_stream_p3_handle = tokio::spawn(run_stream(execution_stream_p3));
    processes.push(execution_stream_p3_handle);

    let execution_stream_p4_handle = tokio::spawn(run_stream(execution_stream_p4));
    processes.push(execution_stream_p4_handle);

    let tx_submission_stream_handle = tokio::spawn(run_stream(tx_submission_stream));
    processes.push(tx_submission_stream_handle);

    let reporting_stream_handle = tokio::spawn(run_stream(reporting_stream));
    processes.push(reporting_stream_handle);

    let tx_tracker_handle = tokio::spawn(tx_tracker_agent.run());
    processes.push(tx_tracker_handle);

    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic(info);
        std::process::exit(1);
    }));

    run_stream(processes).await;
}

fn succinct_tx(tx: LedgerTxEvent<TxViewMut>) -> (TransactionHash, u64) {
    let (LedgerTxEvent::TxApplied { tx, block_number, .. }
    | LedgerTxEvent::TxUnapplied { tx, block_number, .. }) = tx;
    (tx.hash, block_number)
}

fn merge_upstreams(
    xs: impl Stream<Item = (PairId, Channel<Transition<EvolvingCardanoEntity>>)> + Unpin,
    ys: impl Stream<
            Item = (
                PairId,
                Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
            ),
        > + Unpin,
) -> impl Stream<
    Item = (
        PairId,
        Either<
            Channel<
                Transition<
                    Bundled<Either<Baked<AnyOrder, OutputRef>, Baked<AnyPool, OutputRef>>, FinalizedTxOut>,
                >,
            >,
            Channel<OrderUpdate<Bundled<Order, FinalizedTxOut>, Order>>,
        >,
    ),
> {
    stream_select!(
        xs.map(|(p, m)| (p, Either::Left(m.map(|s| s.map(|EvolvingCardanoEntity(e)| e))))),
        ys.map(|(p, m)| (
            p,
            Either::Right(m.map(|upd| match upd {
                OrderUpdate::Created(AtomicCardanoEntity(i)) => OrderUpdate::Created(i),
                OrderUpdate::Eliminated(AtomicCardanoEntity(Bundled(i, _))) => OrderUpdate::Eliminated(i),
            }))
        ))
    )
}

#[derive(Parser)]
#[command(name = "bloom-cardano-agent")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Bloom Off-Chain Agent", long_about = None)]
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
}
