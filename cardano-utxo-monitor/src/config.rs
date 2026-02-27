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

fn default_auto_rollback_slots() -> u64 {
    21600
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSyncConfig {
    pub starting_point: Point,

    /// Number of slots to automatically rollback on restart.
    /// Default: 21600 slots (~6 hours on Cardano mainnet, 1 slot/sec).
    #[serde(default = "default_auto_rollback_slots")]
    pub auto_rollback_slots: u64,

    /// Deprecated: no longer used. Automatic rollback is configured via `autoRollbackSlots`.
    #[serde(default)]
    pub replay_from_point: Option<Point>,

    pub disable_rollbacks_until: Slot,
    pub db_path: String,
}
