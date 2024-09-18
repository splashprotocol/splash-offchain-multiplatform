use std::time::Duration;

use cml_core::Slot;

use cardano_chain_sync::client::Point;
use cardano_explorer::data::ExplorerConfig;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::rocks::RocksConfig;
use spectrum_offchain_cardano::{
    creds::{OperatorCred, OperatorRewardAddress},
    node::NodeConfig,
};

#[derive(serde::Deserialize)]
#[serde(bound = "'de: 'a")]
#[serde(rename_all = "camelCase")]
pub struct AppConfig<'a> {
    pub chain_sync: ChainSyncConfig<'a>,
    pub node: NodeConfig,
    pub tx_submission_buffer_size: usize,
    pub batcher_private_key: &'a str, //todo: store encrypted
    pub explorer: ExplorerConfig<'a>,
    pub reward_address: OperatorRewardAddress,
    pub executor_cred: OperatorCred,
    pub cardano_finalization_delay: Duration,
    pub backlog_capacity: u32,
    pub network_id: NetworkId,
    pub maestro_key_path: &'a str,
    pub order_backlog_config: RocksConfig,
    pub inflation_box_persistence_config: RocksConfig,
    pub poll_factory_persistence_config: RocksConfig,
    pub weighting_poll_persistence_config: RocksConfig,
    pub voting_escrow_persistence_config: RocksConfig,
    pub smart_farm_persistence_config: RocksConfig,
    pub perm_manager_persistence_config: RocksConfig,
    pub funding_box_config: RocksConfig,
    pub genesis_start_time: u64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSyncConfig<'a> {
    pub starting_point: Point,
    pub disable_rollbacks_until: Slot,
    pub replay_from_point: Option<Point>,
    pub db_path: &'a str,
}
