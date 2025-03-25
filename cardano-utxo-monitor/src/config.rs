use cardano_chain_sync::client::Point;
use cml_chain::Slot;
use spectrum_offchain_cardano::node::NodeConfig;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub node: NodeConfig,
    pub tx_tracker_buffer_size: usize,
    pub chain_sync: ChainSyncConfig,
    pub index_path: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSyncConfig {
    pub starting_point: Point,
    pub replay_from_point: Option<Point>,
    pub disable_rollbacks_until: Slot,
    pub db_path: String,
}
