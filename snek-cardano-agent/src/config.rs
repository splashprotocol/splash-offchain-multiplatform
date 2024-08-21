use algebra_core::semigroup::Semigroup;
use bloom_offchain::execution_engine::liquidity_book;
use bloom_offchain::execution_engine::liquidity_book::types::Lovelace;
use bloom_offchain::partitioning::Partitioning;
use bloom_offchain_cardano::integrity::{CheckIntegrity, IntegrityViolations};
use bloom_offchain_cardano::orders::adhoc::AdhocFeeStructure;
use bounded_integer::BoundedU64;
use cardano_chain_sync::client::Point;
use cml_core::Slot;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain_cardano::node::NodeConfig;
use std::time::Duration;

#[derive(serde::Deserialize)]
#[serde(bound = "'de: 'a")]
#[serde(rename_all = "camelCase")]
pub struct AppConfig<'a> {
    pub chain_sync: ChainSyncConfig<'a>,
    pub node: NodeConfig<'a>,
    pub tx_submission_buffer_size: usize,
    pub operator_key: &'a str, //todo: store encrypted
    pub cardano_finalization_delay: Duration,
    pub backlog_capacity: u32,
    pub network_id: NetworkId,
    pub maestro_key_path: &'a str,
    pub execution: ExecutionConfig,
    pub channel_buffer_size: usize,
    pub mempool_buffering_duration: Duration,
    pub ledger_buffering_duration: Duration,
    pub partitioning: Partitioning,
    pub adhoc_fee: AdhocFeeConfig,
}

impl<'a> CheckIntegrity for AppConfig<'a> {
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
        let o2o_violations = if self.execution.o2o_allowed {
            IntegrityViolations::one("O2O allowed".to_string())
        } else {
            IntegrityViolations::empty()
        };
        partitioning_violations.combine(o2o_violations)
    }
}

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdhocFeeConfig {
    pub fixed_fee_lovelace: Lovelace,
    pub relative_fee_percent: BoundedU64<0, 100>,
}

impl From<AdhocFeeConfig> for AdhocFeeStructure {
    fn from(value: AdhocFeeConfig) -> Self {
        Self {
            fixed_fee_lovelace: value.fixed_fee_lovelace,
            relative_fee_percent: value.relative_fee_percent,
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSyncConfig<'a> {
    pub starting_point: Point,
    pub replay_from_point: Option<Point>,
    pub disable_rollbacks_until: Slot,
    pub db_path: &'a str,
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
