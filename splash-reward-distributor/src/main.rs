mod config;
mod constants;
mod context;
pub mod emission;
pub mod engine;
mod entity_index;
mod pipeline;
mod positions;

use crate::config::AppConfig;
use crate::context::RuntimeContext;
use crate::engine::executor::Executor;
use crate::engine::queue::RocksDB;
use crate::engine::verifier::HttpVerifier;
use crate::entity_index::rocksdb::IndexerDB;
use crate::entity_index::update_index_from_mempool_dropped_tx;
use crate::pipeline::event_pipeline;
use crate::positions::PositionIndex;
use async_primitives::beacon::Beacon;
use cardano_chain_sync::atomic_flow::atomic_block_flow;
use cardano_chain_sync::cache::LedgerCacheRocksDB;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_explorer::AnyExplorer;
use clap::Parser;
use cml_chain::transaction::Transaction;
use cml_crypto::TransactionHash;
use futures::channel::mpsc;
use futures::stream::FuturesUnordered;
use log::info;
use spectrum_cardano_lib::constants::{CONWAY_ERA_ID, SAFE_BLOCK_TIME};
use spectrum_offchain_cardano::persistent_index::IndexRocksDB;
use spectrum_offchain_cardano::tx_submission::{tx_submission_agent_stream, TxSubmissionAgent};
use spectrum_offchain_cardano::tx_tracker::new_tx_tracker_bundle;
use spectrum_streaming::run_stream;
use splash_dao_offchain::deployment::{
    DeployedValidators as DaoValidators, ProtocolDeployment as DaoDeployment,
};
use splash_dao_offchain::funding::FundingRepoRocksDB;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing_subscriber::fmt::Subscriber;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    let raw_deployment =
        std::fs::read_to_string(args.dao_deployment_path).expect("Cannot load DAO deployment file");
    let dao_validators: DaoValidators =
        serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting LP indexer ..");

    let explorer = AnyExplorer::new(&config.explorer, config.network_id)
        .await
        .expect("Explorer initialization failed");

    let dao_protocol_deployment = DaoDeployment::unsafe_pull(dao_validators, &explorer).await;

    let chain_sync_cache = Arc::new(Mutex::new(LedgerCacheRocksDB::new(config.chain_sync.db_path)));
    let chain_sync = ChainSyncClient::init(
        Arc::clone(&chain_sync_cache),
        config.node.path.clone(),
        config.node.magic,
        config.chain_sync.starting_point,
    )
    .await
    .expect("ChainSync initialization failed");

    let state_synced = Beacon::relaxed(false);
    let (flow_driver, block_events) = atomic_block_flow(
        Box::pin(chain_sync_stream(chain_sync, state_synced)),
        chain_sync_cache,
    );

    let (failed_txs_snd, failed_txs_recv) = mpsc::channel::<Transaction>(config.tx_submission_buffer_size);
    let (confirmed_txs_snd, confirmed_txs_recv) =
        mpsc::channel::<(TransactionHash, u64)>(config.tx_submission_buffer_size);
    let max_confirmation_delay_blocks = config.event_cache_ttl.as_secs() / SAFE_BLOCK_TIME.as_secs();
    let (tx_tracker_agent, tx_tracker_channel) = new_tx_tracker_bundle(
        confirmed_txs_recv,
        failed_txs_snd,
        config.tx_submission_buffer_size,
        max_confirmation_delay_blocks,
    );
    let (tx_submission_agent, tx_submission_channel) =
        TxSubmissionAgent::<CONWAY_ERA_ID, Transaction, _>::new(
            tx_tracker_channel.clone(),
            config.node.clone(),
            config.tx_submission_buffer_size,
        )
        .await
        .expect("LocalTxSubmission initialization failed");
    let tx_submission_stream = tx_submission_agent_stream(tx_submission_agent);

    let position_index = PositionIndex::new();
    let onchain_index = IndexerDB::new(config.onchain_index_db_path);
    let funding_index = FundingRepoRocksDB::new(config.funding_index_db_path);
    let verifier = HttpVerifier::new(config.verifier_url);
    let ctx = RuntimeContext {};
    let executor = Executor::new(
        position_index.clone(),
        onchain_index.clone(),
        funding_index.clone(),
        tx_submission_channel,
        config.emission,
        verifier,
        ctx.clone(),
    );

    let (failed_tx_hash_snd, failed_tx_hash_recv) =
        mpsc::channel::<TransactionHash>(config.tx_submission_buffer_size);

    let queue = RocksDB::new(config.persistent_queue_db_path);
    let (engine_mailbox_snd, engine_mailbox) = mpsc::channel(1024);
    let engine = engine::Engine::new(
        engine_mailbox,
        queue,
        executor,
        config.engine,
        failed_tx_hash_recv,
    );

    let processes = FuturesUnordered::new();

    let engine_handle = tokio::spawn(engine);
    processes.push(engine_handle);

    let flow_driver_handle = tokio::spawn(flow_driver.run());
    processes.push(flow_driver_handle);

    let utxo_index = IndexRocksDB::new(config.utxo_index_db_path);
    let filter = HashSet::from([dao_protocol_deployment.buffer_wallet.hash()]);

    let mempool_index_handle = tokio::spawn(update_index_from_mempool_dropped_tx(
        failed_txs_recv,
        failed_tx_hash_snd,
        onchain_index.clone(),
        funding_index.clone(),
        utxo_index.clone(),
        ctx.clone(),
    ));
    processes.push(mempool_index_handle);

    let event_pipeline_handle = tokio::spawn(event_pipeline(
        block_events,
        engine_mailbox_snd,
        ctx,
        onchain_index,
        funding_index,
        utxo_index,
        filter,
    ));
    processes.push(event_pipeline_handle);

    let tx_submission_stream_handle = tokio::spawn(run_stream(tx_submission_stream));
    let tx_tracker_handle = tokio::spawn(tx_tracker_agent.run());
    processes.push(tx_tracker_handle);
    processes.push(tx_submission_stream_handle);

    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic(info);
        std::process::exit(1);
    }));

    run_stream(processes).await;
}

#[derive(Parser)]
#[command(name = "splash-lp-indexer")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Splash LP Indexer", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    /// Path to the DAO deployment JSON configuration file .
    #[arg(long)]
    dao_deployment_path: String,
    #[arg(long)]
    dao_tokens_path: String,
    /// Path to the bounds JSON configuration file .
    #[arg(long, short)]
    validation_rules_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
    #[arg(long)]
    host: String,
    #[arg(long)]
    port: u16,
}
