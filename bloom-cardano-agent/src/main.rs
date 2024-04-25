use std::sync::{Arc, Once};

use clap::Parser;
use cml_chain::transaction::Transaction;
use cml_crypto::blake2b256;
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::channel::mpsc;
use futures::stream::select_all;
use futures::{stream_select, Stream, StreamExt};
use log::{info, trace};
use tokio::fs;
use tokio::sync::{broadcast, Mutex};
use tracing_subscriber::fmt::Subscriber;

use bloom_cardano_agent::config::AppConfig;
use bloom_cardano_agent::context::ExecutionContext;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::execution_part_stream;
use bloom_offchain::execution_engine::liquidity_book::{ExecutionCap, TLB};
use bloom_offchain::execution_engine::multi_pair::MultiPair;
use bloom_offchain::execution_engine::storage::kv_store::InMemoryKvStore;
use bloom_offchain::execution_engine::storage::{InMemoryStateIndex, StateIndexTracing};
use bloom_offchain_cardano::bounds::Bounds;
use bloom_offchain_cardano::event_sink::context::HandlerContextProto;
use bloom_offchain_cardano::event_sink::entity_index::InMemoryEntityIndex;
use bloom_offchain_cardano::event_sink::handler::{
    PairUpdateHandler, ProcessingTransaction, SpecializedHandler,
};
use bloom_offchain_cardano::event_sink::order_index::InMemoryOrderIndex;
use bloom_offchain_cardano::event_sink::{AtomicCardanoEntity, EvolvingCardanoEntity};
use bloom_offchain_cardano::execution_engine::backlog::interpreter::SpecializedInterpreterViaRunOrder;
use bloom_offchain_cardano::execution_engine::interpreter::CardanoRecipeInterpreter;
use bloom_offchain_cardano::orders::AnyOrder;
use cardano_chain_sync::cache::LedgerCacheRocksDB;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_chain_sync::event_source::ledger_transactions;
use cardano_explorer::Maestro;
use cardano_mempool_sync::client::LocalTxMonitorClient;
use cardano_mempool_sync::data::MempoolUpdate;
use cardano_mempool_sync::mempool_stream;
use cardano_submit_api::client::LocalTxSubmissionClient;
use spectrum_cardano_lib::constants::BABBAGE_ERA_ID;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::backlog::{BacklogCapacity, HotPriorityBacklog};
use spectrum_offchain::data::order::OrderUpdate;
use spectrum_offchain::data::unique_entity::{EitherMod, StateUpdate};
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
use spectrum_offchain_cardano::tx_submission::{tx_submission_agent_stream, TxSubmissionAgent};

mod config;
mod context;

#[tokio::main]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    let raw_deployment = std::fs::read_to_string(args.deployment_path).expect("Cannot load deployment file");
    let deployment: DeployedValidators =
        serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

    let raw_bounds = std::fs::read_to_string(args.bounds_path).expect("Cannot load bounds file");
    let bounds: Bounds = serde_json::from_str(&raw_bounds).expect("Invalid bounds file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting Off-Chain Agent ..");

    let token = fs::read_to_string(config.maestro_key_path).await.expect("Cannot load maestro token");
    trace!("Maestro token is {}, hash is {}", token, hex::encode(blake2b256(token.as_bytes())));
    
    let explorer = Maestro::new(config.maestro_key_path, config.network_id.into())
        .await
        .expect("Maestro instantiation failed");

    let protocol_deployment = ProtocolDeployment::unsafe_pull(deployment, &explorer).await;

    let chain_sync_cache = Arc::new(Mutex::new(LedgerCacheRocksDB::new(config.chain_sync.db_path)));
    let chain_sync = ChainSyncClient::init(
        Arc::clone(&chain_sync_cache),
        config.node.path,
        config.node.magic,
        config.chain_sync.starting_point,
    )
    .await
    .expect("ChainSync initialization failed");

    // n2c clients:
    let mempool_sync =
        LocalTxMonitorClient::<BabbageTransaction>::connect(config.node.path, config.node.magic)
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

    let (operator_sk, operator_pkh, _operator_addr) =
        operator_creds(config.batcher_private_key, config.node.magic);

    let collateral = pull_collateral(operator_pkh, &explorer)
        .await
        .expect("Couldn't retrieve collateral");

    let (pair_upd_snd_p1, pair_upd_recv_p1) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);
    let (pair_upd_snd_p2, pair_upd_recv_p2) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);
    let (pair_upd_snd_p3, pair_upd_recv_p3) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);
    let (pair_upd_snd_p4, pair_upd_recv_p4) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<EvolvingCardanoEntity>>)>(config.channel_buffer_size);

    let partitioned_pair_upd_snd =
        Partitioned::new([pair_upd_snd_p1, pair_upd_snd_p2, pair_upd_snd_p3, pair_upd_snd_p4]);

    let (spec_upd_snd_p1, spec_upd_recv_p1) = mpsc::channel::<(
        PairId,
        OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>,
    )>(config.channel_buffer_size);
    let (spec_upd_snd_p2, spec_upd_recv_p2) = mpsc::channel::<(
        PairId,
        OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>,
    )>(config.channel_buffer_size);
    let (spec_upd_snd_p3, spec_upd_recv_p3) = mpsc::channel::<(
        PairId,
        OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>,
    )>(config.channel_buffer_size);
    let (spec_upd_snd_p4, spec_upd_recv_p4) = mpsc::channel::<(
        PairId,
        OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>,
    )>(config.channel_buffer_size);

    let partitioned_spec_upd_snd =
        Partitioned::new([spec_upd_snd_p1, spec_upd_snd_p2, spec_upd_snd_p3, spec_upd_snd_p4]);

    let entity_index = Arc::new(Mutex::new(InMemoryEntityIndex::new(
        config.cardano_finalization_delay,
    )));
    let spec_order_index = Arc::new(Mutex::new(InMemoryOrderIndex::new(
        config.cardano_finalization_delay,
    )));
    let handler_context = HandlerContextProto {
        executor_cred: config.executor_cred,
        scripts: ProtocolScriptHashes::from(&protocol_deployment),
        bounds,
    };
    let general_upd_handler = PairUpdateHandler::new(
        partitioned_pair_upd_snd,
        Arc::clone(&entity_index),
        handler_context,
    );
    let spec_upd_handler = SpecializedHandler::new(
        PairUpdateHandler::new(partitioned_spec_upd_snd, entity_index, handler_context),
        spec_order_index,
    );

    let handlers_ledger: Vec<Box<dyn EventHandler<LedgerTxEvent<ProcessingTransaction>>>> = vec![
        Box::new(general_upd_handler.clone()),
        Box::new(spec_upd_handler.clone()),
    ];

    let handlers_mempool: Vec<Box<dyn EventHandler<MempoolUpdate<ProcessingTransaction>>>> =
        vec![Box::new(general_upd_handler), Box::new(spec_upd_handler.clone())];

    let prover = OperatorProver::new(&operator_sk);
    let recipe_interpreter = CardanoRecipeInterpreter;
    let spec_interpreter = SpecializedInterpreterViaRunOrder;
    let context = ExecutionContext {
        time: 0.into(),
        deployment: protocol_deployment,
        execution_cap: config.execution_cap.into(),
        reward_addr: config.reward_address,
        backlog_capacity: BacklogCapacity::from(config.backlog_capacity),
        collateral,
        network_id: config.network_id,
    };
    let multi_book = MultiPair::new::<TLB<AnyOrder, AnyPool, ExUnits>>(context.clone(), "Book");
    let multi_backlog = MultiPair::new::<HotPriorityBacklog<Bundled<ClassicalAMMOrder, FinalizedTxOut>>>(
        context.clone(),
        "Backlog",
    );
    let state_index = StateIndexTracing(InMemoryStateIndex::new());
    let state_cache = InMemoryKvStore::new();

    let (signal_tip_reached_snd, signal_tip_reached_recv) = broadcast::channel(1);

    let execution_stream_p1 = execution_part_stream(
        state_index.clone(),
        state_cache.clone(),
        multi_book.clone(),
        multi_backlog.clone(),
        context.clone(),
        recipe_interpreter,
        spec_interpreter,
        prover,
        merge_upstreams(pair_upd_recv_p1, spec_upd_recv_p1),
        tx_submission_channel.clone(),
        signal_tip_reached_snd.subscribe(),
    );
    let execution_stream_p2 = execution_part_stream(
        state_index.clone(),
        state_cache.clone(),
        multi_book.clone(),
        multi_backlog.clone(),
        context.clone(),
        recipe_interpreter,
        spec_interpreter,
        prover,
        merge_upstreams(pair_upd_recv_p2, spec_upd_recv_p2),
        tx_submission_channel.clone(),
        signal_tip_reached_snd.subscribe(),
    );
    let execution_stream_p3 = execution_part_stream(
        state_index.clone(),
        state_cache.clone(),
        multi_book.clone(),
        multi_backlog.clone(),
        context.clone(),
        recipe_interpreter,
        spec_interpreter,
        prover,
        merge_upstreams(pair_upd_recv_p3, spec_upd_recv_p3),
        tx_submission_channel.clone(),
        signal_tip_reached_snd.subscribe(),
    );
    let execution_stream_p4 = execution_part_stream(
        state_index,
        state_cache,
        multi_book,
        multi_backlog,
        context,
        recipe_interpreter,
        spec_interpreter,
        prover,
        merge_upstreams(pair_upd_recv_p4, spec_upd_recv_p4),
        tx_submission_channel,
        signal_tip_reached_snd.subscribe(),
    );

    let ledger_stream = Box::pin(ledger_transactions(
        chain_sync_cache,
        chain_sync_stream(chain_sync, signal_tip_reached_snd),
        config.chain_sync.disable_rollbacks_until,
        config.chain_sync.replay_from_point,
    ))
    .await
    .map(|ev| match ev {
        LedgerTxEvent::TxApplied { tx, slot } => LedgerTxEvent::TxApplied {
            tx: (hash_transaction_canonical(&tx.body), tx),
            slot,
        },
        LedgerTxEvent::TxUnapplied(tx) => {
            LedgerTxEvent::TxUnapplied((hash_transaction_canonical(&tx.body), tx))
        }
    });
    let mempool_stream = mempool_stream(&mempool_sync, signal_tip_reached_recv).map(|ev| match ev {
        MempoolUpdate::TxAccepted(tx) => {
            MempoolUpdate::TxAccepted((hash_transaction_canonical(&tx.body), tx))
        }
    });

    let process_ledger_events_stream = process_events(ledger_stream, handlers_ledger);
    let process_mempool_events_stream = process_events(mempool_stream, handlers_mempool);

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

fn merge_upstreams(
    xs: impl Stream<Item = (PairId, EitherMod<StateUpdate<EvolvingCardanoEntity>>)> + Unpin,
    ys: impl Stream<Item = (PairId, OrderUpdate<AtomicCardanoEntity, AtomicCardanoEntity>)> + Unpin,
) -> impl Stream<
    Item = (
        PairId,
        Either<
            EitherMod<
                StateUpdate<
                    Bundled<Either<Baked<AnyOrder, OutputRef>, Baked<AnyPool, OutputRef>>, FinalizedTxOut>,
                >,
            >,
            OrderUpdate<Bundled<ClassicalAMMOrder, FinalizedTxOut>, ClassicalAMMOrder>,
        >,
    ),
> {
    stream_select!(
        xs.map(|(p, m)| (p, Either::Left(m.map(|s| s.map(|EvolvingCardanoEntity(e)| e))))),
        ys.map(|(p, m)| (
            p,
            Either::Right(match m {
                OrderUpdate::Created(AtomicCardanoEntity(i)) => OrderUpdate::Created(i),
                OrderUpdate::Eliminated(AtomicCardanoEntity(Bundled(i, _))) => OrderUpdate::Eliminated(i),
            })
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
    bounds_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
}
