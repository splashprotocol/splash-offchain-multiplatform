use std::time::Duration;

use bloom_offchain::execution_engine::liquidity_book;
use cml_core::Slot;

use cardano_chain_sync::client::Point;
use cardano_explorer::data::ExplorerConfig;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain_cardano::creds::{OperatorCred, OperatorRewardAddress};

#[derive(serde::Deserialize)]
#[serde(bound = "'de: 'a")]
#[serde(rename_all = "camelCase")]
pub struct AppConfig<'a> {
    pub chain_sync: ChainSyncConfig<'a>,
    pub node: NodeConfig<'a>,
    pub tx_submission_buffer_size: usize,
    pub operator_key: &'a str, //todo: store encrypted
    pub operator_reward_address: OperatorRewardAddress,
    pub cardano_finalization_delay: Duration,
    pub backlog_capacity: u32,
    pub network_id: NetworkId,
    pub maestro_key_path: &'a str,
    pub execution_cap: ExecutionCap,
    pub channel_buffer_size: usize,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeConfig<'a> {
    pub path: &'a str,
    pub magic: u64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSyncConfig<'a> {
    pub starting_point: Point,
    pub replay_from_point: Option<Point>,
    pub disable_rollbacks_until: Slot,
    pub db_path: &'a str,
}

#[derive(serde::Deserialize)]
pub struct ExecutionCap {
    pub soft: ExUnits,
    pub hard: ExUnits,
}

impl From<ExecutionCap> for liquidity_book::ExecutionCap<ExUnits> {
    fn from(value: ExecutionCap) -> Self {
        Self {
            soft: value.soft,
            hard: value.hard,
        }
    }
}
