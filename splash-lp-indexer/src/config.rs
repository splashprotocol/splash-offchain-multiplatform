use cml_core::Slot;

use cardano_chain_sync::client::Point;
use cardano_explorer::config::ExplorerConfig;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain_cardano::node::NodeConfig;
use splash_yf_offchain::settings::MinLovelacePerHarvest;
use splash_yf_offchain::ve_config::VeConfig;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub chain_sync: ChainSyncConfig,
    pub node: NodeConfig,
    pub network_id: NetworkId,
    pub explorer: ExplorerConfig,
    pub utxo_index_db_path: String,
    pub accounts_db_path: String,
    pub gauges_db_path: String,
    pub confirmation_delay_slots: u64,
    pub events_export_topic: String,
    pub bootstrap_servers: String,
    pub harvest_limits: HarvestLimits,
    pub splash_policy_id_hex: String,
    pub ve_config: VeConfig,
    pub genesis_epoch_start_time: u64,
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
