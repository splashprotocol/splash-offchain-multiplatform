use bounded_integer::BoundedU64;
use clap::Parser;
use cml_chain::block::Block;
use cml_chain::transaction::Transaction;
use cml_multi_era::babbage::BabbageTransaction;
use cml_multi_era::MultiEraBlock;
use either::Either;
use futures::channel::mpsc;
use futures::stream::select_all;
use futures::{stream_select, Stream, StreamExt};
use log::info;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing_subscriber::fmt::Subscriber;

use crate::config::AppConfig;
use crate::context::{ExecutionContext, MakerContext};
use crate::entity::{AtomicCardanoEntity, EvolvingCardanoEntity};
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::execution_part_stream;
use bloom_offchain::execution_engine::funding_effect::FundingEvent;
use bloom_offchain::execution_engine::liquidity_book::hot::HotLB;
use bloom_offchain::execution_engine::liquidity_book::TLB;
use bloom_offchain::execution_engine::multi_pair::MultiPair;
use bloom_offchain::execution_engine::storage::kv_store::{InMemoryKvStore, KvStoreWithTracing};
use bloom_offchain::execution_engine::storage::{InMemoryStateIndex, StateIndexWithTracing};
use bloom_offchain_cardano::bounds::ValidationRules;
use bloom_offchain_cardano::event_sink::context::HandlerContextProto;
use bloom_offchain_cardano::event_sink::entity_index::InMemoryEntityIndex;
use bloom_offchain_cardano::event_sink::handler::{
    FundingEventHandler, PairUpdateHandler, SpecializedHandler,
};
use bloom_offchain_cardano::event_sink::order_index::InMemoryKvIndex;
use bloom_offchain_cardano::event_sink::processed_tx::TxViewAtEraBoundary;
use bloom_offchain_cardano::execution_engine::backlog::interpreter::SpecializedInterpreterViaRunOrder;
use bloom_offchain_cardano::execution_engine::interpreter::CardanoRecipeInterpreter;
use bloom_offchain_cardano::integrity::CheckIntegrity;
use bloom_offchain_cardano::orders::adhoc::AdhocFeeStructure;
use bloom_offchain_cardano::orders::AnyOrder;
use bloom_offchain_cardano::partitioning::select_partition;
use cardano_chain_sync::cache::LedgerCacheRocksDB;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_chain_sync::event_source::ledger_transactions;
use cardano_explorer::Maestro;
use cardano_mempool_sync::client::LocalTxMonitorClient;
use cardano_mempool_sync::data::MempoolUpdate;
use cardano_mempool_sync::mempool_stream;
use spectrum_cardano_lib::constants::CONWAY_ERA_ID;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::transaction::OutboundTransaction;
use spectrum_cardano_lib::{OutputRef, Token};
use spectrum_offchain::backlog::{BacklogCapacity, HotPriorityBacklog};
use spectrum_offchain::data::event::{Channel, StateUpdate};
use spectrum_offchain::data::order::OrderUpdate;
use spectrum_offchain::data::Baked;
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain::partitioning::Partitioned;
use spectrum_offchain::streaming::boxed;
use spectrum_offchain_cardano::collateral::pull_collateral;
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::data::order::ClassicalAMMOrder;
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::data::pool::AnyPool;
use spectrum_offchain_cardano::deployment::{DeployedValidators, ProtocolDeployment, ProtocolScriptHashes};
use spectrum_offchain_cardano::prover::operator::OperatorProver;
use spectrum_offchain_cardano::tx_submission::{
    tx_submission_agent_stream, tx_submission_maestro_stream, TxSubmissionAgent, TxSubmissionChannel,
};
use spectrum_streaming::StreamExt as StreamExt1;

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

    let rollback_in_progress = Arc::new(AtomicBool::new(false));

    let explorer = Maestro::new(config.maestro_key_path, config.network_id.into())
        .await
        .expect("Maestro instantiation failed");

    let protocol_deployment = ProtocolDeployment::unsafe_pull(deployment, &explorer).await;

    let chain_sync_cache = Arc::new(Mutex::new(LedgerCacheRocksDB::new(config.chain_sync.db_path)));
    let chain_sync: ChainSyncClient<MultiEraBlock> = ChainSyncClient::init(
        Arc::clone(&chain_sync_cache),
        config.node.path,
        config.node.magic,
        config.chain_sync.starting_point,
    )
    .await
    .expect("ChainSync initialization failed");

    // n2c clients:
    let mempool_sync = LocalTxMonitorClient::<Transaction>::connect(config.node.path, config.node.magic)
        .await
        .expect("MempoolSync initialization failed");

    // prepare upstreams
    let (snd, recv) = mpsc::channel(1024);
    let tx_submission_channel = TxSubmissionChannel(snd);
    let tx_submission_stream = tx_submission_maestro_stream(recv, explorer);

    let (operator_sk, operator_paycred, collateral_address, funding_addresses) =
        operator_creds(config.operator_key, config.network_id);

    info!(
        "Expecting collateral at {}",
        collateral_address.clone().address().to_bech32(None).unwrap()
    );

    let collateral = pull_collateral(collateral_address, &explorer)
        .await
        .expect("Couldn't retrieve collateral");

    let (pair_upd_snd_p1, pair_upd_recv_p1) =
        mpsc::channel::<(PairId, Channel<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);
    let (pair_upd_snd_p2, pair_upd_recv_p2) =
        mpsc::channel::<(PairId, Channel<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);
    let (pair_upd_snd_p3, pair_upd_recv_p3) =
        mpsc::channel::<(PairId, Channel<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);
    let (pair_upd_snd_p4, pair_upd_recv_p4) =
        mpsc::channel::<(PairId, Channel<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);

    let partitioned_pair_upd_snd =
        Partitioned::new([pair_upd_snd_p1, pair_upd_snd_p2, pair_upd_snd_p3, pair_upd_snd_p4]);

    let (spec_upd_snd_p1, spec_upd_recv_p1) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.channel_buffer_size);
    let (spec_upd_snd_p2, spec_upd_recv_p2) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.channel_buffer_size);
    let (spec_upd_snd_p3, spec_upd_recv_p3) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.channel_buffer_size);
    let (spec_upd_snd_p4, spec_upd_recv_p4) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.channel_buffer_size);

    let partitioned_spec_upd_snd =
        Partitioned::new([spec_upd_snd_p1, spec_upd_snd_p2, spec_upd_snd_p3, spec_upd_snd_p4]);

    let (funding_upd_snd_p1, funding_upd_recv_p1) =
        mpsc::channel::<FundingEvent<FinalizedTxOut>>(config.channel_buffer_size);
    let (funding_upd_snd_p2, funding_upd_recv_p2) =
        mpsc::channel::<FundingEvent<FinalizedTxOut>>(config.channel_buffer_size);
    let (funding_upd_snd_p3, funding_upd_recv_p3) =
        mpsc::channel::<FundingEvent<FinalizedTxOut>>(config.channel_buffer_size);
    let (funding_upd_snd_p4, funding_upd_recv_p4) =
        mpsc::channel::<FundingEvent<FinalizedTxOut>>(config.channel_buffer_size);

    let partitioned_funding_event_snd = Partitioned::new([
        funding_upd_snd_p1,
        funding_upd_snd_p2,
        funding_upd_snd_p3,
        funding_upd_snd_p4,
    ]);

    let entity_index = Arc::new(Mutex::new(InMemoryEntityIndex::new(
        config.cardano_finalization_delay,
    )));
    let spec_order_index = Arc::new(Mutex::new(InMemoryKvIndex::new(
        config.cardano_finalization_delay,
    )));
    let funding_index = Arc::new(Mutex::new(InMemoryKvIndex::new(
        config.cardano_finalization_delay,
    )));
    let handler_context = HandlerContextProto {
        executor_cred: operator_paycred,
        scripts: ProtocolScriptHashes::from(&protocol_deployment),
        adhoc_fee_structure: AdhocFeeStructure::empty(),
        validation_rules,
    };
    let general_upd_handler = PairUpdateHandler::new(
        partitioned_pair_upd_snd,
        Arc::clone(&entity_index),
        handler_context,
    );
    let spec_upd_handler = SpecializedHandler::<_, _, _, Token>::new(
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

    let handlers_ledger: Vec<Box<dyn EventHandler<LedgerTxEvent<TxViewAtEraBoundary>>>> = vec![
        Box::new(general_upd_handler.clone()),
        Box::new(spec_upd_handler.clone()),
        Box::new(funding_event_handler.clone()),
    ];

    let handlers_mempool: Vec<Box<dyn EventHandler<MempoolUpdate<TxViewAtEraBoundary>>>> = vec![
        Box::new(general_upd_handler),
        Box::new(spec_upd_handler),
        Box::new(funding_event_handler),
    ];

    let prover = OperatorProver::new(&operator_sk);
    let recipe_interpreter = CardanoRecipeInterpreter;
    let spec_interpreter = SpecializedInterpreterViaRunOrder;
    let maker_context = MakerContext {
        time: 0.into(),
        execution_conf: config.execution.into(),
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
    };
    let context_p2 = ExecutionContext {
        time: 0.into(),
        deployment: protocol_deployment.clone(),
        reward_addr: funding_addresses[1].clone().into(),
        backlog_capacity: BacklogCapacity::from(config.backlog_capacity),
        collateral: collateral.clone(),
        network_id: config.network_id,
        operator_cred: operator_paycred,
    };
    let context_p3 = ExecutionContext {
        time: 0.into(),
        deployment: protocol_deployment.clone(),
        reward_addr: funding_addresses[2].clone().into(),
        backlog_capacity: BacklogCapacity::from(config.backlog_capacity),
        collateral: collateral.clone(),
        network_id: config.network_id,
        operator_cred: operator_paycred,
    };
    let context_p4 = ExecutionContext {
        time: 0.into(),
        deployment: protocol_deployment,
        reward_addr: funding_addresses[3].clone().into(),
        backlog_capacity: BacklogCapacity::from(config.backlog_capacity),
        collateral,
        network_id: config.network_id,
        operator_cred: operator_paycred,
    };
    if args.hot {
        info!("Running in Hot mode!");
        let multi_book = MultiPair::new::<HotLB<AnyOrder, AnyPool, ExUnits>>(maker_context.clone(), "Book");
        let multi_backlog = MultiPair::new::<HotPriorityBacklog<Bundled<ClassicalAMMOrder, FinalizedTxOut>>>(
            maker_context,
            "Backlog",
        );
        let state_index = InMemoryStateIndex::with_tracing();
        let state_cache = InMemoryKvStore::with_tracing();

        let (signal_tip_reached_snd, signal_tip_reached_recv) = broadcast::channel(1);

        let execution_stream_p1 = execution_part_stream(
            state_index.clone(),
            state_cache.clone(),
            multi_book.clone(),
            multi_backlog.clone(),
            context_p1,
            recipe_interpreter,
            spec_interpreter,
            prover,
            select_partition(
                merge_upstreams(pair_upd_recv_p1, spec_upd_recv_p1),
                config.partitioning.clone(),
            ),
            funding_upd_recv_p1,
            tx_submission_channel.clone(),
            signal_tip_reached_snd.subscribe(),
        );
        let execution_stream_p2 = execution_part_stream(
            state_index.clone(),
            state_cache.clone(),
            multi_book.clone(),
            multi_backlog.clone(),
            context_p2,
            recipe_interpreter,
            spec_interpreter,
            prover,
            select_partition(
                merge_upstreams(pair_upd_recv_p2, spec_upd_recv_p2),
                config.partitioning.clone(),
            ),
            funding_upd_recv_p2,
            tx_submission_channel.clone(),
            signal_tip_reached_snd.subscribe(),
        );
        let execution_stream_p3 = execution_part_stream(
            state_index.clone(),
            state_cache.clone(),
            multi_book.clone(),
            multi_backlog.clone(),
            context_p3,
            recipe_interpreter,
            spec_interpreter,
            prover,
            select_partition(
                merge_upstreams(pair_upd_recv_p3, spec_upd_recv_p3),
                config.partitioning.clone(),
            ),
            funding_upd_recv_p3,
            tx_submission_channel.clone(),
            signal_tip_reached_snd.subscribe(),
        );
        let execution_stream_p4 = execution_part_stream(
            state_index,
            state_cache,
            multi_book,
            multi_backlog,
            context_p4,
            recipe_interpreter,
            spec_interpreter,
            prover,
            select_partition(
                merge_upstreams(pair_upd_recv_p4, spec_upd_recv_p4),
                config.partitioning,
            ),
            funding_upd_recv_p4,
            tx_submission_channel,
            signal_tip_reached_snd.subscribe(),
        );

        let ledger_stream = Box::pin(ledger_transactions(
            chain_sync_cache,
            chain_sync_stream(chain_sync, signal_tip_reached_snd),
            config.chain_sync.disable_rollbacks_until,
            config.chain_sync.replay_from_point,
            rollback_in_progress,
        ))
        .await
        .map(|ev| match ev {
            LedgerTxEvent::TxApplied { tx, slot } => LedgerTxEvent::TxApplied {
                tx: TxViewAtEraBoundary::from(tx),
                slot,
            },
            LedgerTxEvent::TxUnapplied(tx) => LedgerTxEvent::TxUnapplied(TxViewAtEraBoundary::from(tx)),
        });
        let mempool_stream = mempool_stream(&mempool_sync, signal_tip_reached_recv).map(|ev| match ev {
            MempoolUpdate::TxAccepted(tx) => MempoolUpdate::TxAccepted(TxViewAtEraBoundary::from(tx)),
        });

        let process_ledger_events_stream =
            process_events(ledger_stream, handlers_ledger).buffered_within(config.ledger_buffering_duration);
        let process_mempool_events_stream = process_events(mempool_stream, handlers_mempool)
            .buffered_within(config.mempool_buffering_duration);

        let mut app = select_all(vec![
            boxed(process_ledger_events_stream),
            boxed(process_mempool_events_stream),
            boxed(execution_stream_p1),
            boxed(execution_stream_p2),
            boxed(execution_stream_p3),
            boxed(execution_stream_p4),
            boxed(tx_submission_stream),
        ]);

        loop {
            app.select_next_some().await;
        }
    } else {
        let multi_book = MultiPair::new::<TLB<AnyOrder, AnyPool, ExUnits>>(maker_context.clone(), "Book");
        let multi_backlog = MultiPair::new::<HotPriorityBacklog<Bundled<ClassicalAMMOrder, FinalizedTxOut>>>(
            maker_context,
            "Backlog",
        );
        let state_index = InMemoryStateIndex::with_tracing();
        let state_cache = InMemoryKvStore::with_tracing();

        let (signal_tip_reached_snd, signal_tip_reached_recv) = broadcast::channel(1);

        let execution_stream_p1 = execution_part_stream(
            state_index.clone(),
            state_cache.clone(),
            multi_book.clone(),
            multi_backlog.clone(),
            context_p1,
            recipe_interpreter,
            spec_interpreter,
            prover,
            select_partition(
                merge_upstreams(pair_upd_recv_p1, spec_upd_recv_p1),
                config.partitioning.clone(),
            ),
            funding_upd_recv_p1,
            tx_submission_channel.clone(),
            signal_tip_reached_snd.subscribe(),
        );
        let execution_stream_p2 = execution_part_stream(
            state_index.clone(),
            state_cache.clone(),
            multi_book.clone(),
            multi_backlog.clone(),
            context_p2,
            recipe_interpreter,
            spec_interpreter,
            prover,
            select_partition(
                merge_upstreams(pair_upd_recv_p2, spec_upd_recv_p2),
                config.partitioning.clone(),
            ),
            funding_upd_recv_p2,
            tx_submission_channel.clone(),
            signal_tip_reached_snd.subscribe(),
        );
        let execution_stream_p3 = execution_part_stream(
            state_index.clone(),
            state_cache.clone(),
            multi_book.clone(),
            multi_backlog.clone(),
            context_p3,
            recipe_interpreter,
            spec_interpreter,
            prover,
            select_partition(
                merge_upstreams(pair_upd_recv_p3, spec_upd_recv_p3),
                config.partitioning.clone(),
            ),
            funding_upd_recv_p3,
            tx_submission_channel.clone(),
            signal_tip_reached_snd.subscribe(),
        );
        let execution_stream_p4 = execution_part_stream(
            state_index,
            state_cache,
            multi_book,
            multi_backlog,
            context_p4,
            recipe_interpreter,
            spec_interpreter,
            prover,
            select_partition(
                merge_upstreams(pair_upd_recv_p4, spec_upd_recv_p4),
                config.partitioning,
            ),
            funding_upd_recv_p4,
            tx_submission_channel,
            signal_tip_reached_snd.subscribe(),
        );

        let ledger_stream = Box::pin(ledger_transactions(
            chain_sync_cache,
            chain_sync_stream(chain_sync, signal_tip_reached_snd),
            config.chain_sync.disable_rollbacks_until,
            config.chain_sync.replay_from_point,
            rollback_in_progress,
        ))
        .await
        .map(|ev| match ev {
            LedgerTxEvent::TxApplied { tx, slot } => LedgerTxEvent::TxApplied {
                tx: TxViewAtEraBoundary::from(tx),
                slot,
            },
            LedgerTxEvent::TxUnapplied(tx) => LedgerTxEvent::TxUnapplied(TxViewAtEraBoundary::from(tx)),
        });
        let mempool_stream = mempool_stream(&mempool_sync, signal_tip_reached_recv).map(|ev| match ev {
            MempoolUpdate::TxAccepted(tx) => MempoolUpdate::TxAccepted(TxViewAtEraBoundary::from(tx)),
        });

        let process_ledger_events_stream =
            process_events(ledger_stream, handlers_ledger).buffered_within(config.ledger_buffering_duration);
        let process_mempool_events_stream = process_events(mempool_stream, handlers_mempool)
            .buffered_within(config.mempool_buffering_duration);

        let mut app = select_all(vec![
            boxed(process_ledger_events_stream),
            boxed(process_mempool_events_stream),
            boxed(execution_stream_p1),
            boxed(execution_stream_p2),
            boxed(execution_stream_p3),
            boxed(execution_stream_p4),
            boxed(tx_submission_stream),
        ]);

        loop {
            app.select_next_some().await;
        }
    }
}

fn merge_upstreams(
    xs: impl Stream<Item = (PairId, Channel<StateUpdate<EvolvingCardanoEntity>>)> + Unpin,
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
                StateUpdate<
                    Bundled<Either<Baked<AnyOrder, OutputRef>, Baked<AnyPool, OutputRef>>, FinalizedTxOut>,
                >,
            >,
            Channel<OrderUpdate<Bundled<ClassicalAMMOrder, FinalizedTxOut>, ClassicalAMMOrder>>,
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
    #[arg(long)]
    hot: bool,
}
