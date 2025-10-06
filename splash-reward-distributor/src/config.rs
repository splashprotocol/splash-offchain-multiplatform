use cml_core::Slot;
use cml_crypto::Ed25519KeyHash;
use std::time::Duration;

use crate::engine::EngineConfig;
use cardano_chain_sync::client::Point;
use cardano_explorer::config::ExplorerConfig;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain_cardano::node::NodeConfig;
use splash_yf_offchain::{settings::MinLovelacePerHarvest, ve_config::VeConfig};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub chain_sync: ChainSyncConfig,
    pub node: NodeConfig,
    pub network_id: NetworkId,
    pub explorer: ExplorerConfig,
    pub funding_index_db_path: String,
    pub onchain_index_db_path: String,
    pub persistent_queue_db_path: String,
    pub utxo_index_db_path: String,
    pub confirmation_delay_blocks: u64,
    pub events_export_topic: String,
    pub bootstrap_servers: String,
    pub harvest_limits: HarvestLimits,
    pub engine: EngineConfig,
    pub event_cache_ttl: Duration,
    pub tx_submission_buffer_size: usize,
    pub verifier_url: String,
    pub max_number_merkle_tree_snapshots: u8,
    pub splash_policy_id_hex: String,
    pub ve_config: VeConfig,
    pub authorized_executors: Vec<Ed25519KeyHash>,
    pub operator_sk: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSyncConfig {
    pub starting_point: Point,
    pub replay_from_point: Option<Point>,
    pub disable_rollbacks_until: Slot,
    pub db_path: String,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarvestLimits {
    pub minimal_lovelace_per_single_harvest: MinLovelacePerHarvest,
}
