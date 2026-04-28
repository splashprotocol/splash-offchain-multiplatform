use bloom_offchain::execution_engine::liquidity_book;
use bloom_offchain::execution_engine::liquidity_book::core::BaseStepBudget;
use bloom_offchain::partitioning::Partitioning;
use bloom_offchain_cardano::integrity::{CheckIntegrity, IntegrityViolations};
use bloom_offchain_cardano::orders::adhoc::AdhocFeeStructure;
use bounded_integer::BoundedU64;
use cardano_chain_sync::client::Point;
use cardano_explorer::config::ExplorerConfig;
use cml_chain::address::{Address, BaseAddress, EnterpriseAddress};
use cml_chain::certs::Credential;
use cml_core::Slot;
use serde::de::{Error, Unexpected};
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::data::small_vec::SmallVec;
use spectrum_offchain_cardano::creds::OperatorRewardAddress;
use spectrum_offchain_cardano::handler_context::{AllowedAdditionalPaymentDestinations, AuthVerificationKey};
use spectrum_offchain_cardano::node::NodeConfig;
use std::net::SocketAddr;
use std::time::Duration;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub chain_sync: ChainSyncConfig,
    pub node: NodeConfig,
    pub reporting_endpoint: SocketAddr,
    pub tx_submission_buffer_size: usize,
    pub operator_key: String,
    pub service_fee_address: OperatorRewardAddress,
    pub event_cache_ttl: Duration,
    pub backlog_capacity: u32,
    pub network_id: NetworkId,
    pub explorer: ExplorerConfig,
    pub execution: ExecutionConfig,
    #[serde(alias = "channel_buffer_size")]
    pub event_feed_buffer_size: usize,
    pub event_feed_buffering_duration: Duration,
    pub partitioning: Partitioning,
    pub adhoc_fee: AdhocFeeConfig,
    #[serde(default = "default_disable_mempool")]
    pub disable_mempool: bool,
    pub health_listen_addr: Option<SocketAddr>,
}

pub fn allowed_payment_destinations(whitelist: Vec<Address>) -> AllowedAdditionalPaymentDestinations {
    AllowedAdditionalPaymentDestinations(SmallVec::new(whitelist.iter().filter_map(|addr| match addr {
        Address::Base(BaseAddress {
            payment: Credential::PubKey { hash, .. },
            ..
        })
        | Address::Enterprise(EnterpriseAddress {
            payment: Credential::PubKey { hash, .. },
            ..
        }) => Some(*hash),
        _ => None,
    })))
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

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequencingConfig {
    pub session_duration: Slot,
    pub session_settlement: Slot,
    pub disable: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AdhocFeeConfig {
    pub relative_fee_bps: BoundedU64<0, 10000>,
}

impl<'de> serde::Deserialize<'de> for AdhocFeeConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let Some(object) = value.as_object() else {
            return Err(D::Error::invalid_type(
                Unexpected::Other("non-object"),
                &"ad-hoc fee config object",
            ));
        };

        if let Some(raw_bps) = object.get("relativeFeeBps") {
            let bps = raw_bps.as_u64().ok_or_else(|| {
                D::Error::invalid_type(Unexpected::Other("non-integer"), &"integer relativeFeeBps")
            })?;
            return Ok(Self {
                relative_fee_bps: bounded_bps::<D::Error>(bps)?,
            });
        }

        let raw_percent = object
            .get("relativeFeePercent")
            .ok_or_else(|| D::Error::missing_field("relativeFeeBps or legacy relativeFeePercent"))?;
        let bps = percent_value_to_bps::<D::Error>(raw_percent)?;
        Ok(Self {
            relative_fee_bps: bounded_bps::<D::Error>(bps)?,
        })
    }
}

fn bounded_bps<E>(bps: u64) -> Result<BoundedU64<0, 10000>, E>
where
    E: Error,
{
    if bps <= 10_000 {
        Ok(BoundedU64::new_saturating(bps))
    } else {
        Err(E::custom("relative fee must be between 0 and 10000 bps"))
    }
}

fn percent_value_to_bps<E>(value: &serde_json::Value) -> Result<u64, E>
where
    E: Error,
{
    match value {
        serde_json::Value::Number(number) => percent_str_to_bps::<E>(&number.to_string()),
        serde_json::Value::String(value) => percent_str_to_bps::<E>(value),
        _ => Err(E::invalid_type(
            Unexpected::Other("non-number"),
            &"number or string relativeFeePercent",
        )),
    }
}

fn percent_str_to_bps<E>(value: &str) -> Result<u64, E>
where
    E: Error,
{
    let value = value.trim();
    let Some((whole, fractional)) = value.split_once('.') else {
        return value
            .parse::<u64>()
            .ok()
            .and_then(|percent| percent.checked_mul(100))
            .ok_or_else(|| E::custom("relativeFeePercent must be a non-negative decimal"));
    };

    if fractional.len() > 2 {
        return Err(E::custom(
            "relativeFeePercent supports at most two decimal places",
        ));
    }
    let whole = whole
        .parse::<u64>()
        .map_err(|_| E::custom("relativeFeePercent must be a non-negative decimal"))?;
    let fractional = format!("{fractional:0<2}")
        .parse::<u64>()
        .map_err(|_| E::custom("relativeFeePercent must be a non-negative decimal"))?;
    whole
        .checked_mul(100)
        .and_then(|whole_bps| whole_bps.checked_add(fractional))
        .ok_or_else(|| E::custom("relativeFeePercent must be a non-negative decimal"))
}

impl From<AdhocFeeConfig> for AdhocFeeStructure {
    fn from(value: AdhocFeeConfig) -> Self {
        Self {
            relative_fee_bps: value.relative_fee_bps,
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

impl ExecutionConfig {
    pub fn into_lb_config(
        self,
        base_step_budget: BaseStepBudget,
    ) -> liquidity_book::config::ExecutionConfig<ExUnits> {
        liquidity_book::config::ExecutionConfig {
            execution_cap: self.execution_cap.into(),
            o2o_allowed: false,
            base_step_budget,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adhoc_fee_config_accepts_basis_points() {
        let config: AdhocFeeConfig =
            serde_json::from_str(r#"{"relativeFeeBps":130}"#).expect("config must parse");

        assert_eq!(config.relative_fee_bps.get(), 130);
        assert_eq!(AdhocFeeStructure::from(config).fee(1_000_000_000), 13_000_000);
    }

    #[test]
    fn adhoc_fee_config_maps_legacy_percent_to_basis_points() {
        let config: AdhocFeeConfig =
            serde_json::from_str(r#"{"relativeFeePercent":1}"#).expect("config must parse");

        assert_eq!(config.relative_fee_bps.get(), 100);
    }

    #[test]
    fn adhoc_fee_config_accepts_decimal_percent() {
        let config: AdhocFeeConfig =
            serde_json::from_str(r#"{"relativeFeePercent":1.3}"#).expect("config must parse");

        assert_eq!(config.relative_fee_bps.get(), 130);
    }

    #[test]
    fn adhoc_fee_config_rejects_more_than_two_decimal_places() {
        let error = serde_json::from_str::<AdhocFeeConfig>(r#"{"relativeFeePercent":1.333}"#)
            .expect_err("config must reject sub-bps precision");

        assert!(error.to_string().contains("at most two decimal places"));
    }
}
