use bloom_offchain::execution_engine::liquidity_book;
use bloom_offchain::partitioning::Partitioning;
use bloom_offchain_cardano::integrity::{CheckIntegrity, IntegrityViolations};
use bloom_offchain_cardano::orders::adhoc::AdhocFeeStructure;
use bounded_integer::BoundedU64;
use cardano_chain_sync::client::Point;
use cml_core::Slot;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain_cardano::creds::OperatorRewardAddress;
use spectrum_offchain_cardano::handler_context::AuthVerificationKey;
use spectrum_offchain_cardano::node::NodeConfig;
use std::time::Duration;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub chain_sync: ChainSyncConfig,
    pub node: NodeConfig,
    pub tx_submission_buffer_size: usize,
    pub operator_key: String,
    pub auth_verification_key: AuthVerificationKey,
    pub service_fee_address: OperatorRewardAddress,
    pub cardano_finalization_delay: Duration,
    pub backlog_capacity: u32,
    pub network_id: NetworkId,
    pub maestro_key_path: String,
    pub execution: ExecutionConfig,
    pub channel_buffer_size: usize,
    pub mempool_buffering_duration: Duration,
    pub ledger_buffering_duration: Duration,
    pub partitioning: Partitioning,
    pub adhoc_fee: AdhocFeeConfig,
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

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdhocFeeConfig {
    pub relative_fee_percent: BoundedU64<0, 100>,
}

impl From<AdhocFeeConfig> for AdhocFeeStructure {
    fn from(value: AdhocFeeConfig) -> Self {
        Self {
            relative_fee_percent: value.relative_fee_percent,
        }
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
}

impl From<ExecutionConfig> for liquidity_book::config::ExecutionConfig<ExUnits> {
    fn from(conf: ExecutionConfig) -> Self {
        Self {
            execution_cap: conf.execution_cap.into(),
            o2o_allowed: false,
        }
    }
}
