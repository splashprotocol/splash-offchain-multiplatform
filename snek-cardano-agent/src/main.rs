use clap::Parser;
use cml_chain::transaction::Transaction;
use cml_multi_era::MultiEraBlock;
use either::Either;
use futures::channel::mpsc;
use futures::stream::FuturesUnordered;
use futures::{stream_select, Stream, StreamExt};
use log::info;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing_subscriber::fmt::Subscriber;

use crate::config::AppConfig;
use crate::context::{ExecutionContext, MakerContext};
use crate::entity::{AtomicCardanoEntity, EvolvingCardanoEntity};
use crate::snek_handler_context::{SnekHandlerContext, SnekHandlerContextProto};
use crate::snek_protocol_deployment::{
    SnekDeployedValidators, SnekProtocolDeployment, SnekProtocolScriptHashes,
};
use crate::snek_validation_rules::SnekValidationRules;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::execution_part_stream;
use bloom_offchain::execution_engine::funding_effect::FundingEvent;
use bloom_offchain::execution_engine::liquidity_book::hot::HotLB;
use bloom_offchain::execution_engine::liquidity_book::TLB;
use bloom_offchain::execution_engine::multi_pair::MultiPair;
use bloom_offchain::execution_engine::storage::InMemoryStateIndex;
use bloom_offchain_cardano::event_sink::entity_index::InMemoryEntityIndex;
use bloom_offchain_cardano::event_sink::handler::{FundingEventHandler, PairUpdateHandler};
use bloom_offchain_cardano::event_sink::order_index::InMemoryKvIndex;
use bloom_offchain_cardano::event_sink::processed_tx::TxViewAtEraBoundary;
use bloom_offchain_cardano::execution_engine::backlog::interpreter::SpecializedInterpreterViaRunOrder;
use bloom_offchain_cardano::execution_engine::interpreter::CardanoRecipeInterpreter;
use bloom_offchain_cardano::integrity::CheckIntegrity;
use bloom_offchain_cardano::orders::adhoc::AdhocOrder;
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
use spectrum_offchain::streaming::run_stream;
use spectrum_offchain_cardano::collateral::pull_collateral;
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::data::degen_quadratic_pool::DegenQuadraticPool;
use spectrum_offchain_cardano::data::order::ClassicalAMMOrder;
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::prover::operator::OperatorProver;
use spectrum_offchain_cardano::tx_submission::{tx_submission_agent_stream, TxSubmissionAgent};

mod config;
mod context;
mod entity;
mod snek_handler_context;
mod snek_protocol_deployment;
mod snek_validation_rules;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::File::open(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_reader(raw_config).expect("Invalid configuration file");
    let config_integrity_violations = config.check_integrity();
    if !config_integrity_violations.is_empty() {
        panic!("Malformed configuration: {}", config_integrity_violations);
    }

    let raw_deployment = std::fs::read_to_string(args.deployment_path).expect("Cannot load deployment file");
    let deployment: SnekDeployedValidators =
        serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

    let raw_validation_rules =
        std::fs::read_to_string(args.validation_rules_path).expect("Cannot load bounds file");
    let validation_rules: SnekValidationRules =
        serde_json::from_str(&raw_validation_rules).expect("Invalid bounds file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting Snek Agent ..");

    let rollback_in_progress = Arc::new(AtomicBool::new(false));

    let explorer = Maestro::new(config.maestro_key_path, config.network_id.into())
        .await
        .expect("Maestro instantiation failed");

    let protocol_deployment = SnekProtocolDeployment::unsafe_pull(deployment, &explorer).await;

    let chain_sync_cache = Arc::new(Mutex::new(LedgerCacheRocksDB::new(config.chain_sync.db_path)));
    let chain_sync: ChainSyncClient<MultiEraBlock> = ChainSyncClient::init(
        Arc::clone(&chain_sync_cache),
        config.node.path.clone(),
        config.node.magic,
        config.chain_sync.starting_point,
    )
    .await
    .expect("ChainSync initialization failed");

    // n2c clients:
    let mempool_sync =
        LocalTxMonitorClient::<Transaction>::connect(config.node.path.as_str(), config.node.magic)
            .await
            .expect("MempoolSync initialization failed");
    let (tx_submission_agent, tx_submission_channel) =
        TxSubmissionAgent::<CONWAY_ERA_ID, OutboundTransaction<Transaction>, Transaction>::new(
            config.node.clone(),
            config.tx_submission_buffer_size,
        )
        .await
        .expect("LocalTxSubmission initialization failed");

    // prepare upstreams
    let tx_submission_stream = tx_submission_agent_stream(tx_submission_agent);

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
        mpsc::channel::<(PairId, Channel<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);
    let (pair_upd_snd_p2, pair_upd_recv_p2) =
        mpsc::channel::<(PairId, Channel<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);
    let (pair_upd_snd_p3, pair_upd_recv_p3) =
        mpsc::channel::<(PairId, Channel<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);
    let (pair_upd_snd_p4, pair_upd_recv_p4) =
        mpsc::channel::<(PairId, Channel<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);

    let partitioned_pair_upd_snd =
        Partitioned::new([pair_upd_snd_p1, pair_upd_snd_p2, pair_upd_snd_p3, pair_upd_snd_p4]);

    let (_, spec_upd_recv_p1) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.channel_buffer_size);
    let (_, spec_upd_recv_p2) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.channel_buffer_size);
    let (_, spec_upd_recv_p3) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.channel_buffer_size);
    let (_, spec_upd_recv_p4) = mpsc::channel::<(
        PairId,
        Channel<OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>>,
    )>(config.channel_buffer_size);

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
    let funding_index = Arc::new(Mutex::new(InMemoryKvIndex::new(
        config.cardano_finalization_delay,
    )));
    let handler_context = SnekHandlerContextProto {
        executor_cred: operator_paycred,
        scripts: SnekProtocolScriptHashes::from(&protocol_deployment),
        validation_rules,
        adhoc_fee_structure: config.adhoc_fee.into(),
        auth_verification_key: config.auth_verification_key,
    };
    let general_upd_handler: PairUpdateHandler<
        4,
        _,
        _,
        _,
        _,
        SnekHandlerContextProto,
        SnekHandlerContext<Token>,
    > = PairUpdateHandler::new(
        partitioned_pair_upd_snd,
        Arc::clone(&entity_index),
        handler_context,
    );
    let funding_event_handler = FundingEventHandler::new(
        partitioned_funding_event_snd,
        funding_addresses.clone(),
        collateral.reference(), // collateral cannot be used for funding.
        funding_index,
    );

    info!("Derived funding addresses: {}", funding_addresses);

    let handlers_ledger: Vec<Box<dyn EventHandler<LedgerTxEvent<TxViewAtEraBoundary>> + Send>> = vec![
        Box::new(general_upd_handler.clone()),
        Box::new(funding_event_handler.clone()),
    ];

    let handlers_mempool: Vec<Box<dyn EventHandler<MempoolUpdate<TxViewAtEraBoundary>> + Send>> =
        vec![Box::new(general_upd_handler), Box::new(funding_event_handler)];

    let prover = OperatorProver::new(config.operator_key);
    let recipe_interpreter = CardanoRecipeInterpreter::new(true);
    let spec_interpreter = SpecializedInterpreterViaRunOrder;
    let maker_context = MakerContext {
        time: 0.into(),
        execution_conf: config.execution.into(),
        backlog_capacity: BacklogCapacity::from(config.backlog_capacity),
    };
    let context = ExecutionContext {
        time: 0.into(),
        deployment: protocol_deployment.clone(),
        reward_addr: config.service_fee_address,
        backlog_capacity: BacklogCapacity::from(config.backlog_capacity),
        collateral: collateral.clone(),
        network_id: config.network_id,
        operator_cred: operator_paycred,
        adhoc_fee_structure: config.adhoc_fee.into(),
    };
    let (signal_tip_reached_snd, signal_tip_reached_recv) = broadcast::channel(1);
    if args.hot {
        let multi_book =
            MultiPair::new::<HotLB<AdhocOrder, DegenQuadraticPool, ExUnits>>(maker_context.clone(), "Book");
        let multi_backlog = MultiPair::new::<HotPriorityBacklog<Bundled<ClassicalAMMOrder, FinalizedTxOut>>>(
            maker_context,
            "Backlog",
        );
        let state_index = InMemoryStateIndex::with_tracing();

        let execution_stream_p1 = execution_part_stream(
            state_index.clone(),
            multi_book.clone(),
            multi_backlog.clone(),
            context.clone(),
            recipe_interpreter,
            spec_interpreter,
            prover.clone(),
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
            multi_book.clone(),
            multi_backlog.clone(),
            context.clone(),
            recipe_interpreter,
            spec_interpreter,
            prover.clone(),
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
            multi_book.clone(),
            multi_backlog.clone(),
            context.clone(),
            recipe_interpreter,
            spec_interpreter,
            prover.clone(),
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
            multi_book,
            multi_backlog,
            context,
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

        let mempool_stream = mempool_stream(mempool_sync, signal_tip_reached_recv).map(|ev| match ev {
            MempoolUpdate::TxAccepted(tx) => MempoolUpdate::TxAccepted(TxViewAtEraBoundary::from(tx)),
        });

        let process_ledger_events_stream = process_events(ledger_stream, handlers_ledger);
        let process_mempool_events_stream = process_events(mempool_stream, handlers_mempool);

        let processes = FuturesUnordered::new();

        let process_ledger_events_stream_handle = tokio::spawn(run_stream(process_ledger_events_stream));
        processes.push(process_ledger_events_stream_handle);

        let process_mempool_events_stream_handle = tokio::spawn(run_stream(process_mempool_events_stream));
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

        run_stream(processes).await;
    } else {
        let multi_book =
            MultiPair::new::<TLB<AdhocOrder, DegenQuadraticPool, ExUnits>>(maker_context.clone(), "Book");
        let multi_backlog = MultiPair::new::<HotPriorityBacklog<Bundled<ClassicalAMMOrder, FinalizedTxOut>>>(
            maker_context,
            "Backlog",
        );
        let state_index = InMemoryStateIndex::with_tracing();

        let execution_stream_p1 = execution_part_stream(
            state_index.clone(),
            multi_book.clone(),
            multi_backlog.clone(),
            context.clone(),
            recipe_interpreter,
            spec_interpreter,
            prover.clone(),
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
            multi_book.clone(),
            multi_backlog.clone(),
            context.clone(),
            recipe_interpreter,
            spec_interpreter,
            prover.clone(),
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
            multi_book.clone(),
            multi_backlog.clone(),
            context.clone(),
            recipe_interpreter,
            spec_interpreter,
            prover.clone(),
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
            multi_book,
            multi_backlog,
            context,
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

        let mempool_stream = mempool_stream(mempool_sync, signal_tip_reached_recv).map(|ev| match ev {
            MempoolUpdate::TxAccepted(tx) => MempoolUpdate::TxAccepted(TxViewAtEraBoundary::from(tx)),
        });

        let process_ledger_events_stream = process_events(ledger_stream, handlers_ledger);
        let process_mempool_events_stream = process_events(mempool_stream, handlers_mempool);

        let processes = FuturesUnordered::new();

        let process_ledger_events_stream_handle = tokio::spawn(run_stream(process_ledger_events_stream));
        processes.push(process_ledger_events_stream_handle);

        let process_mempool_events_stream_handle = tokio::spawn(run_stream(process_mempool_events_stream));
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

        run_stream(processes).await;
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
                    Bundled<
                        Either<Baked<AdhocOrder, OutputRef>, Baked<DegenQuadraticPool, OutputRef>>,
                        FinalizedTxOut,
                    >,
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
#[command(name = "snek-cardano-agent")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Snek Off-Chain Agent", long_about = None)]
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
