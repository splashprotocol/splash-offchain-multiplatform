use std::sync::{Arc, Once};

use clap::Parser;
use cml_chain::genesis::network_info::NetworkInfo;
use cml_chain::transaction::Transaction;
use cml_multi_era::babbage::BabbageTransaction;
use futures::channel::mpsc;
use log::info;
use tokio::sync::Mutex;
use tracing_subscriber::fmt::Subscriber;

use bloom_offchain_cardano::event_sink::entity_index::InMemoryEntityIndex;
use bloom_offchain_cardano::event_sink::handler::PairUpdateHandler;
use bloom_offchain_cardano::event_sink::CardanoEvent;
use bloom_offchain_cardano::PairId;
use cardano_chain_sync::chain_sync_stream;
use cardano_chain_sync::client::ChainSyncClient;
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_chain_sync::event_source::event_source_ledger;
use cardano_explorer::client::Explorer;
use cardano_mempool_sync::client::LocalTxMonitorClient;
use cardano_mempool_sync::data::MempoolUpdate;
use cardano_mempool_sync::mempool_stream;
use cardano_submit_api::client::LocalTxSubmissionClient;
use spectrum_cardano_lib::constants::BABBAGE_ERA_ID;
use spectrum_offchain::data::unique_entity::{EitherMod, StateUpdate};
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain::partitioning::Partitioned;
use spectrum_offchain_cardano::collaterals::{Collaterals, CollateralsViaExplorer};
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::data::ref_scripts::ReferenceOutputs;
use spectrum_offchain_cardano::tx_submission::{tx_submission_agent_stream, TxSubmissionAgent};

use crate::config::AppConfig;

mod config;

#[tokio::main]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting Off-Chain Agent ..");

    let explorer = Explorer::new(config.explorer);

    let ref_scripts = ReferenceOutputs::pull(config.ref_scripts, explorer)
        .await
        .expect("Ref scripts initialization failed");

    let chain_sync = ChainSyncClient::init(
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
    let signal_tip_reached = Once::new();
    let ledger_stream = Box::pin(event_source_ledger(chain_sync_stream(
        chain_sync,
        Some(&signal_tip_reached),
    )));
    let mempool_stream = mempool_stream(&mempool_sync, Some(&signal_tip_reached));

    let (operator_sk, operator_pkh, operator_addr) =
        operator_creds(config.batcher_private_key, NetworkInfo::mainnet());

    let collaterals = CollateralsViaExplorer::new(operator_pkh.to_hex(), explorer);

    let collateral = collaterals
        .get_collateral()
        .await
        .expect("Couldn't retrieve collateral");

    let (pair_upd_snd_p1, pair_upd_recv_p1) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<CardanoEvent>>)>(128);
    let (pair_upd_snd_p2, pair_upd_recv_p2) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<CardanoEvent>>)>(128);
    let (pair_upd_snd_p3, pair_upd_recv_p3) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<CardanoEvent>>)>(128);
    let (pair_upd_snd_p4, pair_upd_recv_p4) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<CardanoEvent>>)>(128);

    let partitioned_pair_upd_snd =
        Partitioned::new([pair_upd_snd_p1, pair_upd_snd_p2, pair_upd_snd_p3, pair_upd_snd_p4]);
    let index = Arc::new(Mutex::new(InMemoryEntityIndex::new()));
    let upd_handler = PairUpdateHandler::new(partitioned_pair_upd_snd, index);

    let handlers_ledger: Vec<Box<dyn EventHandler<LedgerTxEvent<BabbageTransaction>>>> =
        vec![Box::new(upd_handler.clone())];

    let handlers_mempool: Vec<Box<dyn EventHandler<MempoolUpdate<BabbageTransaction>>>> =
        vec![Box::new(upd_handler)];

    let process_ledger_events_stream = process_events(ledger_stream, handlers_ledger);
    let process_mempool_events_stream = process_events(mempool_stream, handlers_mempool);
}

const NUM_PART: usize = 4;

#[derive(Parser)]
#[command(name = "bloom-cardano-agent")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Bloom Off-Chain Agent", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
}
