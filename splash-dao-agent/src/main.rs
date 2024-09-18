use std::{
    marker::PhantomData,
    sync::{atomic::AtomicBool, Arc, Once},
    time::UNIX_EPOCH,
};

use async_trait::async_trait;
use bloom_offchain_cardano::event_sink::processed_tx::TxViewAtEraBoundary;
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
use cml_crypto::TransactionHash;
use cml_multi_era::babbage::BabbageTransaction;
use config::AppConfig;
use futures::{stream::select_all, FutureExt, Stream, StreamExt};
use spectrum_cardano_lib::{
    constants::BABBAGE_ERA_ID, hash::hash_transaction_canonical, output::FinalizedTxOut,
    transaction::OutboundTransaction, NetworkId,
};
use spectrum_offchain::{
    backlog::{persistence::BacklogStoreRocksDB, BacklogConfig, PersistentPriorityBacklog},
    event_sink::{event_handler::EventHandler, process_events},
    rocks::RocksConfig,
    streaming::boxed,
};
use spectrum_offchain_cardano::{
    collateral::pull_collateral,
    creds::{operator_creds, operator_creds_base_address},
    prover::operator::OperatorProver,
    tx_submission::{tx_submission_agent_stream, TxSubmissionAgent},
};
use splash_dao_offchain::{
    deployment::{DaoDeployment, ProtocolDeployment},
    entities::offchain::voting_order::VotingOrder,
    handler::DaoHandler,
    protocol_config::{ProtocolConfig, ProtocolTokens},
    routines::inflation::{actions::CardanoInflationActions, Behaviour},
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
    let deployment: DaoDeployment = serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting DAO Agent ..");

    let rollback_in_progress = Arc::new(AtomicBool::new(false));

    let explorer = Maestro::new(config.maestro_key_path, config.network_id.into())
        .await
        .expect("Maestro instantiation failed");
    let protocol_deployment = ProtocolDeployment::unsafe_pull(deployment.validators, &explorer).await;

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
    let (tx_submission_agent, tx_submission_channel) =
        TxSubmissionAgent::<BABBAGE_ERA_ID, OutboundTransaction<Transaction>, Transaction>::new(
            config.node,
            config.tx_submission_buffer_size,
        )
        .await
        .unwrap();

    // prepare upstreams
    let tx_submission_stream = tx_submission_agent_stream(tx_submission_agent);
    let (signal_tip_reached_snd, mut signal_tip_reached_recv) = tokio::sync::broadcast::channel(1);
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

    // We assume the batcher's private key is associated with a Cardano base address, which also
    // includes a reward address.
    let (addr, _, operator_pkh, operator_cred, operator_sk) =
        operator_creds_base_address(config.batcher_private_key, config.network_id);

    let reward_address = RewardAddress::new(
        NetworkInfo::preprod().network_id(),
        StakeCredential::new_pub_key(operator_sk.to_public().hash()),
    );

    let collateral = pull_collateral(addr.into(), &explorer)
        .await
        .expect("Couldn't retrieve collateral");

    let node_magic: u8 = config.network_id.into();
    let protocol_config = ProtocolConfig {
        deployed_validators: protocol_deployment,
        tokens: ProtocolTokens::from_minted_tokens(deployment.nfts),
        operator_sk: config.batcher_private_key.into(),
        network_id: config.network_id,
        node_magic: node_magic as u64,
        reward_address,
        collateral,
        genesis_time: config.genesis_start_time.into(),
    };

    let (ledger_event_snd, ledger_event_rcv) = tokio::sync::mpsc::channel(100);

    let inflation_actions = CardanoInflationActions::from(protocol_config.clone());

    let mut behaviour = Behaviour::new(
        StateProjectionRocksDB::new(config.inflation_box_persistence_config),
        StateProjectionRocksDB::new(config.poll_factory_persistence_config),
        StateProjectionRocksDB::new(config.weighting_poll_persistence_config),
        StateProjectionRocksDB::new(config.voting_escrow_persistence_config),
        StateProjectionRocksDB::new(config.smart_farm_persistence_config),
        StateProjectionRocksDB::new(config.perm_manager_persistence_config),
        StateProjectionRocksDB::new(config.funding_box_config),
        setup_order_backlog(config.order_backlog_config).await,
        NetworkTimeSource {},
        inflation_actions,
        protocol_config,
        PhantomData::<TransactionOutput>,
        tx_submission_channel,
        operator_sk,
        ledger_event_rcv,
        signal_tip_reached_recv,
    );

    let handlers: Vec<Box<dyn EventHandler<LedgerTxEvent<TxViewAtEraBoundary>>>> =
        vec![Box::new(DaoHandler::new(ledger_event_snd))];
    let process_ledger_events_stream = process_events(ledger_stream, handlers);

    let mut app = select_all(vec![
        boxed(process_ledger_events_stream),
        boxed(behaviour.as_stream()),
        boxed(tx_submission_stream),
    ]);
    loop {
        app.select_next_some().await;
    }
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

//fn receiver_as_stream<T>(rcv: tokio::sync::mpsc::Receiver<T>) -> impl Stream<Item = T> {
//    stream! {
//        loop {
//            match rcv.
//        }
//    }
//}
