use cml_core::serialization::RawBytesEncoding;
use std::net::SocketAddr;
use std::time::Duration;

use cml_core::Slot;
use cml_crypto::PublicKey;

use bloom_offchain::execution_engine::liquidity_book;
use bloom_offchain::execution_engine::liquidity_book::core::BaseStepBudget;
use bloom_offchain::partitioning::Partitioning;
use cardano_chain_sync::client::Point;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain_cardano::node::NodeConfig;

use bloom_offchain_cardano::integrity::{CheckIntegrity, IntegrityViolations};
use spectrum_offchain::data::small_vec::SmallVec;
use spectrum_offchain_cardano::data::dao_request::DAOContext;
use spectrum_offchain_cardano::data::royalty_withdraw_request::RoyaltyWithdrawContext;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub chain_sync: ChainSyncConfig,
    pub node: NodeConfig,
    pub reporting_endpoint: SocketAddr,
    pub tx_submission_buffer_size: usize,
    pub operator_key: String,
    pub take_residual_fee: bool,
    pub event_cache_ttl: Duration,
    pub backlog_capacity: u32,
    pub network_id: NetworkId,
    pub maestro_key_path: String,
    pub execution: ExecutionConfig,
    #[serde(alias = "channel_buffer_size")]
    pub event_feed_buffer_size: usize,
    pub event_feed_buffering_duration: Duration,
    pub partitioning: Partitioning,
    pub dao_config: DAOConfig,
    pub royalty_withdraw: RoyaltyWithdrawContext,
    #[serde(default = "default_disable_mempool")]
    pub disable_mempool: bool,
}

fn default_disable_mempool() -> bool {
    false
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

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DAOConfig {
    pub public_keys: Vec<String>,
    pub threshold: u64,
    pub ex_fee: u64,
}

impl Into<DAOContext> for DAOConfig {
    fn into(self) -> DAOContext {
        let parsed_public_keys: Vec<[u8; 32]> = self
            .public_keys
            .into_iter()
            .try_fold(vec![], |mut acc, key_to_parse| {
                PublicKey::from_raw_bytes(&hex::decode(key_to_parse).unwrap()).map(|parsed_key| {
                    acc.push(parsed_key.to_raw_bytes().try_into().unwrap());
                    acc
                })
            })
            .unwrap();

        DAOContext {
            public_keys: SmallVec::new(parsed_public_keys.into_iter()),
            signature_threshold: self.threshold,
            execution_fee: self.ex_fee,
        }
    }
}
