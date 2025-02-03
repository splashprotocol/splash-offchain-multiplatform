use std::{fmt::Debug, hash::Hash, marker::PhantomData, sync::Arc};

use async_primitives::beacon::Beacon;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use bloom_offchain_cardano::event_sink::processed_tx::TxViewMut;
use bounded_integer::BoundedU8;
use cardano_chain_sync::{
    cache::LedgerCacheRocksDB, chain_sync_stream, client::ChainSyncClient, data::LedgerTxEvent,
    event_source::ledger_transactions,
};
use cardano_explorer::Maestro;
use chrono::Duration;
use clap::Parser;
use cml_chain::{
    address::RewardAddress,
    certs::StakeCredential,
    transaction::{Transaction, TransactionOutput},
};
use cml_crypto::TransactionHash;
use cml_multi_era::babbage::BabbageTransaction;
use config::AppConfig;
use futures::{channel::mpsc, stream::FuturesUnordered, StreamExt};
use serde::de::DeserializeOwned;
use spectrum_cardano_lib::constants::{CONWAY_ERA_ID, SAFE_BLOCK_TIME};
use spectrum_offchain::{
    backlog::{data::Weighted, persistence::BacklogStoreRocksDB, BacklogConfig, PersistentPriorityBacklog},
    domain::order::UniqueOrder,
    event_sink::{
        event_handler::{forward_to, EventHandler},
        process_events,
    },
    kv_store::KVStoreRocksDB,
    rocks::RocksConfig,
};
use spectrum_offchain_cardano::{
    creds::operator_creds,
    tx_submission::{tx_submission_agent_stream, TxSubmissionAgent},
    tx_tracker::new_tx_tracker_bundle,
};
use spectrum_streaming::run_stream;
use splash_dao_offchain::{
    collateral::pull_collateral,
    constants::DAO_SCRIPT_BYTES,
    deployment::{CompleteDeployment, DaoScriptData, DeploymentProgress, ProtocolDeployment},
    entities::{
        offchain::{extend_voting_escrow_order::ExtendVotingEscrowOffChainOrder, voting_order::VotingOrder},
        onchain::{make_voting_escrow_order::MakeVotingEscrowOrderBundle, voting_escrow::Owner},
    },
    funding::FundingRepoRocksDB,
    handler::DaoHandler,
    protocol_config::ProtocolConfig,
    routines::inflation::{
        actions::CardanoInflationActions, Behaviour, DaoBotCommand, DaoBotMessage, DaoBotResponse,
        VotingOrderCommand, VotingOrderStatus,
    },
    state_projection::StateProjectionRocksDB,
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

    let dao_script_bytes_str =
        std::fs::read_to_string(args.script_bytes_path).expect("Cannot load script bytes file");
    let dao_script_bytes: DaoScriptData =
        serde_json::from_str(&dao_script_bytes_str).expect("Invalid script bytes file");

    DAO_SCRIPT_BYTES.set(dao_script_bytes).unwrap();

    let deployment = CompleteDeployment::try_from((deployment_progress, config.network_id)).unwrap();

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting DAO Agent ..");

    let state_synced = Beacon::relaxed(false);
    let rollback_in_progress = Beacon::strong(false);

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

    // We assume the batcher's private key is associated with a Cardano base address, which also
    // includes a reward address.
    let (addr, collateral_addr, _funding_addresses) =
        operator_creds(config.batcher_private_key, config.network_id);

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
        .route("/submit/votingorder", axum::routing::put(handle_voting_put))
        .route("/submit/extendve", axum::routing::put(handle_extend_ve_put))
        .route(
            "/query/ve/identifier/name",
            axum::routing::put(handle_get_mve_status),
        )
        .with_state(state);
    println!("Listening on {}", config.voting_order_listener_endpoint);

    let listener = tokio::net::TcpListener::bind(config.voting_order_listener_endpoint)
        .await
        .unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let inflation_actions = CardanoInflationActions::from(protocol_config.clone());

    let mk_path = |name: &str| {
        if name.ends_with('/') {
            format!("{}{}", config.persistence_stores_root_dir, name)
        } else {
            format!("{}/{}", config.persistence_stores_root_dir, name)
        }
    };

    let mut behaviour = Behaviour::new(
        StateProjectionRocksDB::new(mk_path("inflation_box")),
        StateProjectionRocksDB::new(mk_path("poll_factory")),
        StateProjectionRocksDB::new(mk_path("weighting_poll")),
        StateProjectionRocksDB::new(mk_path("ve_factory")),
        StateProjectionRocksDB::new(mk_path("voting_escrow")),
        StateProjectionRocksDB::new(mk_path("smart_farm")),
        StateProjectionRocksDB::new(mk_path("perm_manager")),
        FundingRepoRocksDB::new(mk_path("funding_box")),
        setup_order_backlog(mk_path("make_voting_escrow_owner")).await,
        setup_order_backlog(mk_path("extend_voting_escrow_owner")).await,
        KVStoreRocksDB::new(mk_path("voting_escrow_by_owner")),
        KVStoreRocksDB::new(mk_path("tx_hash_to_mve")),
        setup_order_backlog(mk_path("voting_order_backlog_config")).await,
        setup_order_backlog(mk_path("extend_ve_order_backlog_config")).await,
        KVStoreRocksDB::new(mk_path("predicted_txs_backlog")),
        NetworkTimeSource {},
        inflation_actions,
        protocol_config,
        PhantomData::<TransactionOutput>,
        tx_submission_channel,
        ledger_event_rcv,
        voting_event_rcv,
        state_synced,
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

async fn handle_voting_put(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<VotingOrder>,
) -> impl IntoResponse {
    let AppState { sender } = state;
    let (response_sender, recv) = tokio::sync::oneshot::channel();
    let msg = DaoBotMessage {
        command: DaoBotCommand::VotingOrder(VotingOrderCommand::Submit(payload)),
        response_sender,
    };
    sender.send(msg).await.unwrap();
    match recv.await {
        Ok(response) => match response {
            DaoBotResponse::VotingOrder(voting_order_status) => match voting_order_status {
                VotingOrderStatus::Queued | VotingOrderStatus::Success => {
                    (StatusCode::OK, format!("{:?}", voting_order_status))
                }
                VotingOrderStatus::Failed => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "TX submission failed".into())
                }
                VotingOrderStatus::VotingEscrowNotFound => (
                    StatusCode::NOT_FOUND,
                    "Cannot find associated voting_escrow".into(),
                ),
            },
            DaoBotResponse::MVEStatus(mve_status) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Unexpected response: MVEStatus: {:?}", mve_status),
            ),
        },
        Err(_err) => (StatusCode::UNPROCESSABLE_ENTITY, "Unknown error".into()),
    }
}

async fn handle_extend_ve_put(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<ExtendVotingEscrowOffChainOrder>,
) -> impl IntoResponse {
    let AppState { sender } = state;
    let (response_sender, recv) = tokio::sync::oneshot::channel();
    let msg = DaoBotMessage {
        command: DaoBotCommand::ExtendVotingEscrowOrder(payload),
        response_sender,
    };
    sender.send(msg).await.unwrap();
    match recv.await {
        Ok(response) => match response {
            DaoBotResponse::VotingOrder(voting_order_status) => match voting_order_status {
                VotingOrderStatus::Queued | VotingOrderStatus::Success => {
                    (StatusCode::OK, format!("{:?}", voting_order_status))
                }
                VotingOrderStatus::Failed => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "TX submission failed".into())
                }
                VotingOrderStatus::VotingEscrowNotFound => (
                    StatusCode::NOT_FOUND,
                    "Cannot find associated voting_escrow".into(),
                ),
            },
            DaoBotResponse::MVEStatus(mve_status) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Unexpected response: MVEStatus: {:?}", mve_status),
            ),
        },
        Err(_err) => (StatusCode::UNPROCESSABLE_ENTITY, "Unknown error".into()),
    }
}

async fn handle_get_mve_status(
    State(state): State<AppState>,
    axum::Json(owner): axum::Json<Owner>,
) -> impl IntoResponse {
    let AppState { sender } = state;
    let (response_sender, recv) = tokio::sync::oneshot::channel();
    let msg = DaoBotMessage {
        command: DaoBotCommand::GetMVEOrderStatus {
            mve_order_owner: owner,
        },
        response_sender,
    };
    sender.send(msg).await.unwrap();
    match recv.await {
        Ok(status) => match status {
            DaoBotResponse::MVEStatus(status) => (StatusCode::OK, Json(Some(status))),
            DaoBotResponse::VotingOrder(_) => (StatusCode::UNPROCESSABLE_ENTITY, Json(None)),
        },
        Err(_err) => (StatusCode::UNPROCESSABLE_ENTITY, Json(None)),
    }
}

fn succinct_tx(tx: LedgerTxEvent<TxViewMut>) -> (TransactionHash, u64) {
    let (LedgerTxEvent::TxApplied { tx, block_number, .. }
    | LedgerTxEvent::TxUnapplied { tx, block_number, .. }) = tx;
    (tx.hash, block_number)
}

#[derive(Clone)]
struct AppState {
    sender: tokio::sync::mpsc::Sender<DaoBotMessage>,
}

async fn setup_order_backlog<T>(db_path: String) -> PersistentPriorityBacklog<T, BacklogStoreRocksDB>
where
    T: Hash + Eq + UniqueOrder + Weighted + serde::Serialize + DeserializeOwned + Send + 'static,
    T::TOrderId: Debug + serde::Serialize + DeserializeOwned + Send,
{
    let store = BacklogStoreRocksDB::new(RocksConfig { db_path });
    let backlog_config = BacklogConfig {
        order_lifespan: Duration::try_hours(72).unwrap(),
        order_exec_time: Duration::try_hours(72).unwrap(),
        retry_suspended_prob: BoundedU8::new(60).unwrap(),
    };

    PersistentPriorityBacklog::new::<T>(store, backlog_config).await
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
    /// Path to the JSON DAO script bytes file.
    #[arg(long, short)]
    script_bytes_path: String,
    /// Path to the JSON deployment configuration file .
    #[arg(long, short)]
    deployment_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
}

pub type ProcessingTransaction = (TransactionHash, BabbageTransaction);
