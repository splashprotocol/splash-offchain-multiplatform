use std::time::Duration;

use cml_core::Slot;

use bloom_offchain::execution_engine::liquidity_book;
use bloom_offchain::partitioning::Partitioning;
use cardano_chain_sync::client::Point;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain_cardano::node::NodeConfig;

use bloom_offchain_cardano::integrity::{CheckIntegrity, IntegrityViolations};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub chain_sync: ChainSyncConfig,
    pub node: NodeConfig,
    pub tx_submission_buffer_size: usize,
    pub operator_key: String,
    pub take_residual_fee: bool,
    pub cardano_finalization_delay: Duration,
    pub backlog_capacity: u32,
    pub network_id: NetworkId,
    pub maestro_key_path: String,
    pub execution: ExecutionConfig,
    pub channel_buffer_size: usize,
    pub event_feed_buffering_duration: Duration,
    pub partitioning: Partitioning,
}

impl CheckIntegrity for AppConfig {
    fn check_integrity(&self) -> IntegrityViolations {
        let partitioning_violations = if self
            .partitioning
            .assigned_partitions
            .iter()
            .all(|p| *p < self.partitioning.num_partitions_total)
        {
            IntegrityViolations::empty()
        } else {
            IntegrityViolations::one("Bad partitioning".to_string())
        };
        partitioning_violations
    }
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

impl From<ExecutionConfig> for liquidity_book::config::ExecutionConfig<ExUnits> {
    fn from(conf: ExecutionConfig) -> Self {
        Self {
            execution_cap: conf.execution_cap.into(),
            o2o_allowed: conf.o2o_allowed,
        }
    }
}
