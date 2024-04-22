use std::sync::{Arc, Once};

use cardano_chain_sync::{
    cache::LedgerCacheRocksDB, chain_sync_stream, client::ChainSyncClient, event_source::ledger_transactions,
};
use cardano_explorer::client::Explorer;
use cardano_submit_api::client::LocalTxSubmissionClient;
use clap::Parser;
use cml_chain::transaction::Transaction;
use config::AppConfig;
use spectrum_cardano_lib::constants::BABBAGE_ERA_ID;
use spectrum_offchain_cardano::{
    collaterals::{Collaterals, CollateralsViaExplorer},
    creds::operator_creds,
    tx_submission::{tx_submission_agent_stream, TxSubmissionAgent},
};
use splash_dao_offchain::deployment::{DeployedValidators, ProtocolDeployment};
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
    let deployment: DeployedValidators =
        serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting DAO Agent ..");

    let explorer = Explorer::new(config.explorer, config.node.magic);
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
    let tx_submission_client =
        LocalTxSubmissionClient::<BABBAGE_ERA_ID, Transaction>::init(config.node.path, config.node.magic)
            .await
            .expect("LocalTxSubmission initialization failed");
    let (tx_submission_agent, tx_submission_channel) =
        TxSubmissionAgent::new(tx_submission_client, config.tx_submission_buffer_size);

    // prepare upstreams
    let tx_submission_stream = tx_submission_agent_stream(tx_submission_agent);
    let signal_tip_reached = Once::new();
    let ledger_stream = Box::pin(ledger_transactions(
        chain_sync_cache,
        chain_sync_stream(chain_sync, Some(&signal_tip_reached)),
        config.chain_sync.disable_rollbacks_until,
    ));

    let (operator_sk, operator_pkh, _operator_addr) =
        operator_creds(config.batcher_private_key, config.node.magic);

    let collaterals = CollateralsViaExplorer::new(operator_pkh.to_hex(), explorer);

    let collateral = collaterals
        .get_collateral()
        .await
        .expect("Couldn't retrieve collateral");
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
