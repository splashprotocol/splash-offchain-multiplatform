use std::{
    i64,
    marker::PhantomData,
    sync::{atomic::AtomicBool, Arc, Once},
    time::UNIX_EPOCH,
};

use async_trait::async_trait;
use axum::{extract::State, http::StatusCode, response::IntoResponse};
use bloom_offchain_cardano::event_sink::processed_tx::TxViewMut;
use bounded_integer::BoundedU8;
use cardano_chain_sync::{
    cache::LedgerCacheRocksDB, chain_sync_stream, client::ChainSyncClient, data::LedgerTxEvent,
    event_source::ledger_transactions,
};
use cardano_explorer::Maestro;
use cardano_submit_api::client::LocalTxSubmissionClient;
use chrono::Duration;
use clap::Parser;
use cml_chain::{
    address::RewardAddress,
    assets::AssetName,
    certs::StakeCredential,
    genesis::network_info::NetworkInfo,
    transaction::{Transaction, TransactionOutput},
    PolicyId,
};
use cml_crypto::{Bip32PrivateKey, TransactionHash};
use cml_multi_era::babbage::BabbageTransaction;
use config::AppConfig;
use futures::{
    channel::mpsc,
    stream::{select_all, FuturesUnordered},
    FutureExt, Stream, StreamExt,
};
use spectrum_cardano_lib::{
    constants::{BABBAGE_ERA_ID, CONWAY_ERA_ID, SAFE_BLOCK_TIME},
    hash::hash_transaction_canonical,
    output::FinalizedTxOut,
    NetworkId,
};
use spectrum_offchain::{
    backlog::{persistence::BacklogStoreRocksDB, BacklogConfig, PersistentPriorityBacklog},
    event_sink::{
        event_handler::{forward_to, EventHandler},
        process_events,
    },
    kv_store::KVStoreRocksDB,
    rocks::RocksConfig,
    streaming::{boxed, run_stream},
};
use spectrum_offchain_cardano::{
    creds::{operator_creds, operator_creds_base_address},
    prover::operator::OperatorProver,
    tx_submission::{tx_submission_agent_stream, TxSubmissionAgent},
    tx_tracker::new_tx_tracker_bundle,
};
use splash_dao_offchain::{
    collateral::pull_collateral,
    deployment::{CompleteDeployment, DeploymentProgress, ProtocolDeployment},
    entities::{
        offchain::voting_order::VotingOrder,
        onchain::make_voting_escrow_order::{MakeVotingEscrowOrder, MakeVotingEscrowOrderBundle},
    },
    funding::FundingRepoRocksDB,
    handler::DaoHandler,
    protocol_config::ProtocolConfig,
    routines::inflation::{
        actions::CardanoInflationActions, Behaviour, PredictedEntityWrites, VotingOrderCommand,
        VotingOrderMessage, VotingOrderStatus,
    },
    state_projection::StateProjectionRocksDB,
    time::{NetworkTime, NetworkTimeProvider},
    NetworkTimeSource,
};
use tokio::sync::Mutex;
use tracing::info;
use tracing_subscriber::fmt::Subscriber;

mod config;

#[tokio::main]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    let raw_deployment = std::fs::read_to_string(args.deployment_path).expect("Cannot load deployment file");
    let deployment_progress: DeploymentProgress =
        serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

    let deployment = CompleteDeployment::try_from((deployment_progress, config.network_id)).unwrap();

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting DAO Agent ..");

    let rollback_in_progress = Arc::new(AtomicBool::new(false));

    let explorer = Maestro::new(config.maestro_key_path, config.network_id.into())
        .await
        .expect("Maestro instantiation failed");
    let protocol_deployment =
        ProtocolDeployment::unsafe_pull(deployment.deployed_validators, &explorer).await;

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
    let (failed_txs_snd, failed_txs_recv) =
        tokio::sync::mpsc::channel::<Transaction>(config.tx_submission_buffer_size);
    let (confirmed_txs_snd, confirmed_txs_recv) =
        mpsc::channel::<(TransactionHash, u64)>(config.tx_submission_buffer_size);
    let max_confirmation_delay_blocks = config.event_cache_ttl.as_secs() / SAFE_BLOCK_TIME.as_secs();
    info!("max_confirmation_delay_blocks: {}", max_confirmation_delay_blocks);
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

    // prepare upstreams
    let tx_submission_stream = tx_submission_agent_stream(tx_submission_agent);
    let (signal_tip_reached_snd, signal_tip_reached_recv) = tokio::sync::broadcast::channel(1);
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

    // We assume the batcher's private key is associated with a Cardano base address, which also
    // includes a reward address.
    let (addr, collateral_addr, funding_addresses) =
        operator_creds(config.batcher_private_key, config.network_id);

    let operator_sk = Bip32PrivateKey::from_bech32(config.batcher_private_key)
        .expect("wallet error")
        .to_raw_key();

    let reward_address = RewardAddress::new(config.network_id.into(), StakeCredential::new_pub_key(addr.0));

    let collateral = pull_collateral(collateral_addr, &explorer)
        .await
        .expect("Couldn't retrieve collateral");

    let node_magic: u8 = config.network_id.into();
    let protocol_config = ProtocolConfig {
        deployed_validators: protocol_deployment,
        tokens: deployment.minted_deployment_tokens,
        operator_sk: config.batcher_private_key.into(),
        network_id: config.network_id,
        node_magic: node_magic as u64,
        reward_address,
        collateral,
        genesis_time: deployment.genesis_epoch_start_time.into(),
    };

    let (ledger_event_snd, ledger_event_rcv) = tokio::sync::mpsc::channel(100);
    let (voting_order_snd, voting_event_rcv) = tokio::sync::mpsc::channel(100);

    let state = AppState {
        sender: voting_order_snd,
    };

    // Setup axum server to listen for incoming voting orders --------------------------------------
    let app = axum::Router::new()
        .route("/submit/votingorder", axum::routing::put(handle_put))
        .with_state(state);
    println!("Listening on {}", config.voting_order_listener_endpoint);

    let listener = tokio::net::TcpListener::bind(config.voting_order_listener_endpoint)
        .await
        .unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let inflation_actions = CardanoInflationActions::from(protocol_config.clone());

    let mut behaviour = Behaviour::new(
        StateProjectionRocksDB::new(config.inflation_box_persistence_config),
        StateProjectionRocksDB::new(config.poll_factory_persistence_config),
        StateProjectionRocksDB::new(config.weighting_poll_persistence_config),
        StateProjectionRocksDB::new(config.ve_factory_persistence_config),
        StateProjectionRocksDB::new(config.voting_escrow_persistence_config),
        StateProjectionRocksDB::new(config.smart_farm_persistence_config),
        StateProjectionRocksDB::new(config.perm_manager_persistence_config),
        FundingRepoRocksDB::new(config.funding_box_config.db_path),
        setup_make_ve_order_backlog(config.make_voting_escrow_owner_config).await,
        setup_order_backlog(config.order_backlog_config).await,
        KVStoreRocksDB::new(config.predicted_txs_backlog_config.db_path),
        NetworkTimeSource {},
        inflation_actions,
        protocol_config,
        PhantomData::<TransactionOutput>,
        tx_submission_channel,
        ledger_event_rcv,
        voting_event_rcv,
        signal_tip_reached_recv,
        failed_txs_recv,
    );

    let handlers: Vec<Box<dyn EventHandler<LedgerTxEvent<TxViewMut>> + Send>> = vec![
        Box::new(DaoHandler::new(ledger_event_snd)),
        Box::new(forward_to(confirmed_txs_snd, succinct_tx)),
    ];
    let process_ledger_events_stream = process_events(ledger_stream, handlers);

    let processes = FuturesUnordered::new();

    let process_ledger_events_stream_handle = tokio::spawn(run_stream(process_ledger_events_stream));
    processes.push(process_ledger_events_stream_handle);

    let behaviour_stream_handle = tokio::spawn(async move {
        behaviour.as_stream().collect::<Vec<_>>().await;
    });
    processes.push(behaviour_stream_handle);

    let tx_submission_stream_handle = tokio::spawn(run_stream(tx_submission_stream));
    processes.push(tx_submission_stream_handle);

    let tx_tracker_handle = tokio::spawn(tx_tracker_agent.run());
    processes.push(tx_tracker_handle);

    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic(info);
        std::process::exit(1);
    }));

    run_stream(processes).await;
}

async fn handle_put(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<VotingOrder>,
) -> impl IntoResponse {
    // You can now process the payload as needed
    let AppState { sender } = state;
    let (response_sender, recv) = tokio::sync::oneshot::channel();
    let msg = VotingOrderMessage {
        command: VotingOrderCommand::Submit(payload),
        response_sender,
    };
    sender.send(msg).await.unwrap();
    match recv.await {
        Ok(status) => match status {
            VotingOrderStatus::Queued | VotingOrderStatus::Success => {
                (StatusCode::OK, format!("{:?}", status))
            }
            VotingOrderStatus::Failed => (StatusCode::UNPROCESSABLE_ENTITY, "TX submission failed".into()),
            VotingOrderStatus::VotingEscrowNotFound => (
                StatusCode::NOT_FOUND,
                "Cannot find associated voting_escrow".into(),
            ),
        },
        Err(_err) => (StatusCode::UNPROCESSABLE_ENTITY, "Unknown error".into()),
    }
}

fn succinct_tx(tx: LedgerTxEvent<TxViewMut>) -> (TransactionHash, u64) {
    let (LedgerTxEvent::TxApplied { tx, block_number, .. }
    | LedgerTxEvent::TxUnapplied { tx, block_number, .. }) = tx;
    (tx.hash, block_number)
}

#[derive(Clone)]
struct AppState {
    sender: tokio::sync::mpsc::Sender<VotingOrderMessage>,
}

async fn setup_order_backlog(
    store_conf: RocksConfig,
) -> PersistentPriorityBacklog<VotingOrder, BacklogStoreRocksDB> {
    let store = BacklogStoreRocksDB::new(store_conf);
    let backlog_config = BacklogConfig {
        order_lifespan: Duration::try_hours(1).unwrap(),
        order_exec_time: Duration::try_minutes(5).unwrap(),
        retry_suspended_prob: BoundedU8::new(60).unwrap(),
    };

    PersistentPriorityBacklog::new::<VotingOrder>(store, backlog_config).await
}

async fn setup_make_ve_order_backlog(
    store_conf: RocksConfig,
) -> PersistentPriorityBacklog<MakeVotingEscrowOrderBundle<TransactionOutput>, BacklogStoreRocksDB> {
    let store = BacklogStoreRocksDB::new(store_conf);
    let backlog_config = BacklogConfig {
        order_lifespan: Duration::try_hours(72).unwrap(),
        order_exec_time: Duration::try_hours(72).unwrap(),
        retry_suspended_prob: BoundedU8::new(60).unwrap(),
    };

    PersistentPriorityBacklog::new::<MakeVotingEscrowOrderBundle<TransactionOutput>>(store, backlog_config)
        .await
}

#[derive(Parser)]
#[command(name = "splash-dao-agent")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Splash DAO Agent", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    /// Path to the JSON deployment configuration file .
    #[arg(long, short)]
    deployment_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
}

pub type ProcessingTransaction = (TransactionHash, BabbageTransaction);
