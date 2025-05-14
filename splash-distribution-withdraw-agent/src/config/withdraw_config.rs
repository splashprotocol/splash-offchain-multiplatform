use cardano_chain_sync::client::Point;
use cardano_explorer::config::ExplorerConfig;
use cml_core::Slot;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain_cardano::deployment::DeployedValidatorRef;
use spectrum_offchain_cardano::node::NodeConfig;
use splash_dao_offchain::deployment::IssuedAsset;
use std::time::Duration;
use cml_chain::PolicyId;
use splash_lp_indexer::config::HarvestLimits;
use crate::onchain::event::BufferWalletAddress;

#[derive(serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawConfig {
    pub chain_sync: ChainSyncConfig,
    pub db_path: String,
    pub buffered_wallets_db: String,
    pub position_db: String,
    pub buffered_wallets_storage: String,
    pub smart_farm_storage: String,
    pub users_withdraw_storage: String,
    pub perm_manager_storage: String,
    pub node: NodeConfig,
    pub tx_submission_buffer_size: usize,
    pub utxo_index_db_path: String,
    pub event_cache_ttl: Duration,
    pub harvest_order: DeployedValidatorRef,
    pub perm_manager: DeployedValidatorRef,
    pub smart_farm: DeployedValidatorRef,
    pub distributor_key: String,
    pub network_id: NetworkId,
    pub explorer: ExplorerConfig,
    pub harvest_limits: HarvestLimits,
    pub persistence_stores_root_dir: String,
    pub perm_auth: IssuedAsset,
    pub splash_policy_id: PolicyId,
    pub buffered_wallet: String
}

#[derive(serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChainSyncConfig {
    pub starting_point: Point,
    pub replay_from_point: Option<Point>,
    pub disable_rollbacks_until: Slot,
    pub db_path: String,
}
