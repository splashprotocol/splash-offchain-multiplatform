use cml_core::Slot;

use bloom_offchain::execution_engine::liquidity_book;
use bloom_offchain::execution_engine::liquidity_book::core::BaseStepBudget;
use cardano_chain_sync::client::Point;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain_cardano::node::NodeConfig;

use spectrum_offchain_cardano::data::pool::PoolValidation;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub chain_sync: ChainSyncConfig,
    pub node: NodeConfig,
    pub network_id: NetworkId,
    pub maestro_key_path: String,
    pub event_feed_buffer_size: usize,
    pub pool_validation: PoolValidation,
    pub utxo_index_db_path: String,
    pub event_log_db_path: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSyncConfig {
    pub starting_point: Point,
    pub replay_from_point: Option<Point>,
    pub disable_rollbacks_until: Slot,
    pub db_path: String,
}

#[derive(Copy, Clone, serde::Deserialize)]
pub struct ExecutionCap {
    pub soft: ExUnits,
    pub hard: ExUnits,
}

impl From<ExecutionCap> for liquidity_book::config::ExecutionCap<ExUnits> {
    fn from(value: ExecutionCap) -> Self {
        Self {
            soft: value.soft,
            hard: value.hard,
        }
    }
}

#[derive(Copy, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionConfig {
    pub execution_cap: ExecutionCap,
    /// Order-order matchmaking allowed.
    pub o2o_allowed: bool,
}

impl ExecutionConfig {
    pub fn into_lb_config(
        self,
        base_step_budget: BaseStepBudget,
    ) -> liquidity_book::config::ExecutionConfig<ExUnits> {
        liquidity_book::config::ExecutionConfig {
            execution_cap: self.execution_cap.into(),
            o2o_allowed: self.o2o_allowed,
            base_step_budget,
        }
    }
}
