use clap::Parser;
use futures::StreamExt;
use serde::Deserialize;
use tracing_subscriber::fmt::Subscriber;

use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::{ChainSyncClient, ChainSyncConf};
use cardano_chain_sync::event_source::event_source_ledger;
use cardano_submit_api::client::{LocalTxSubmissionClient, LocalTxSubmissionConf};
use spectrum_cardano_lib::constants::BABBAGE_ERA_ID;
use spectrum_offchain::backlog::HotPriorityBacklog;
use spectrum_offchain::box_resolver::persistence::EphemeralEntityRepo;

use crate::data::order::ClassicalOnChainOrder;
use crate::prover::noop::NoopProver;
use crate::tx_submission::TxSubmissionAgent;

mod constants;
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
    let config: AppConfig = serde_yaml::from_str(&raw_config).expect("Invalid configuration file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    let chain_sync = ChainSyncClient::init(config.chain_sync)
        .await
        .expect("ChainSync initialization failed");
    let tx_submission_client = LocalTxSubmissionClient::<BABBAGE_ERA_ID>::init(config.local_tx_submission)
        .await
        .expect("LocalTxSubmission initialization failed");
    let (tx_submission_agent, tx_submission_channel) = TxSubmissionAgent::new(tx_submission_client, config.tx_submission_buffer_size);
    let mut ledger_stream = Box::pin(event_source_ledger(chain_sync_stream(chain_sync)));
    let backlog = HotPriorityBacklog::<ClassicalOnChainOrder>::new(43);
    let pool_repo = EphemeralEntityRepo::new();
    let prover = NoopProver {};
}

#[derive(Deserialize)]
#[serde(bound = "'de: 'a")]
struct AppConfig<'a> {
    chain_sync: ChainSyncConf<'a>,
    local_tx_submission: LocalTxSubmissionConf<'a>,
    tx_submission_buffer_size: usize,
}

#[derive(Parser)]
#[command(name = "spectrum-offchain-cardano")]
#[command(author = "Spectrum Finance")]
#[command(version = "1.0.0")]
#[command(about = "Spectrum DEX Offchain Bot", long_about = None)]
struct AppArgs {
    /// Path to the YAML configuration file.
    #[arg(long, short)]
    config_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
}
