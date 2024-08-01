use std::collections::HashSet;
use std::fmt::{Display, Formatter, Write};
use std::time::Duration;

use cml_chain::address::{Address, BaseAddress};
use cml_chain::certs::{Credential, StakeCredential};
use cml_core::Slot;

use crate::integrity::{CheckIntegrity, IntegrityViolations};
use algebra_core::semigroup::Semigroup;
use bloom_offchain::execution_engine::liquidity_book;
use bloom_offchain::partitioning::Partitioning;
use bloom_offchain_cardano::funding::FundingAddresses;
use cardano_chain_sync::client::Point;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain_cardano::node::NodeConfig;

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
    pub execution_cap: ExecutionCap,
    pub channel_buffer_size: usize,
    pub mempool_buffering_duration: Duration,
    pub ledger_buffering_duration: Duration,
    pub partitioning: Partitioning,
    pub funding: FundingConfig,
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
        self.funding.check_integrity().combine(partitioning_violations)
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

/// Stake credentials of funding addresses for all 4 partitions.
/// Each credential must be unique.
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingConfig {
    pub stake_credential_pt1: StakeCredential,
    pub stake_credential_pt2: StakeCredential,
    pub stake_credential_pt3: StakeCredential,
    pub stake_credential_pt4: StakeCredential,
}

impl FundingConfig {
    pub fn into_funding_addresses(
        self,
        network_id: NetworkId,
        payment_credential: Credential,
    ) -> FundingAddresses<4> {
        let FundingConfig {
            stake_credential_pt1,
            stake_credential_pt2,
            stake_credential_pt3,
            stake_credential_pt4,
        } = self;
        [
            Address::Base(BaseAddress::new(
                network_id.into(),
                payment_credential.clone(),
                stake_credential_pt1,
            )),
            Address::Base(BaseAddress::new(
                network_id.into(),
                payment_credential.clone(),
                stake_credential_pt2,
            )),
            Address::Base(BaseAddress::new(
                network_id.into(),
                payment_credential.clone(),
                stake_credential_pt3,
            )),
            Address::Base(BaseAddress::new(
                network_id.into(),
                payment_credential,
                stake_credential_pt4,
            )),
        ]
        .into()
    }
}

impl CheckIntegrity for FundingConfig {
    fn check_integrity(&self) -> IntegrityViolations {
        let this = self.clone();
        let set = HashSet::from([
            this.stake_credential_pt1,
            this.stake_credential_pt2,
            this.stake_credential_pt3,
            this.stake_credential_pt4,
        ]);
        if set.len() == 4 {
            IntegrityViolations::empty()
        } else {
            IntegrityViolations::one("Each funding credential must be unique".to_string())
        }
    }
}

#[derive(Copy, Clone, serde::Deserialize)]
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
