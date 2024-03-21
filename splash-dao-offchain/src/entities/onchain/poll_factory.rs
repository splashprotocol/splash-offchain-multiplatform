use std::fmt::Formatter;

use cml_chain::plutus::{ConstrPlutusData, ExUnits, PlutusData};

use cml_chain::PolicyId;
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::Token;
use spectrum_offchain::data::{Identifier, Stable};

use crate::entities::onchain::smart_farm::FarmId;
use crate::entities::onchain::weighting_poll::WeightingPoll;
use crate::routines::inflation::PollFactorySnapshot;
use crate::time::ProtocolEpoch;

use super::weighting_poll::WeightingPollStableId;

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd)]
pub struct PollFactoryId(Token);

impl Identifier for PollFactoryId {
    type For = PollFactorySnapshot;
}

pub struct PollFactory {
    pub last_poll_epoch: ProtocolEpoch,
    pub active_farms: Vec<FarmId>,
    pub stable_id: PollFactoryStableId,
}

impl PollFactory {
    pub fn next_epoch(&self) -> ProtocolEpoch {
        self.last_poll_epoch + 1
    }
    pub fn next_weighting_poll(mut self, farm_auth_policy: PolicyId) -> (PollFactory, WeightingPoll) {
        let poll_epoch = self.last_poll_epoch + 1;
        let stable_id = WeightingPollStableId {
            auth_policy: self.stable_id.wp_auth_policy,
            farm_auth_policy,
        };
        let next_poll = WeightingPoll {
            epoch: poll_epoch,
            distribution: self.active_farms.iter().map(|farm| (*farm, 0u64)).collect(),
            stable_id,
        };
        self.last_poll_epoch = poll_epoch;
        (self, next_poll)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PollFactoryStableId {
    /// Auth policy of all weighting polls.
    pub wp_auth_policy: PolicyId,
    /// Hash of the Governance Proxy witness.
    pub gov_witness_script_hash: PolicyId,
}

impl std::fmt::Display for PollFactoryStableId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "wp_auth_policy: {}, gov_witness_script_hash: {}",
            self.wp_auth_policy, self.gov_witness_script_hash
        ))
    }
}

impl Stable for PollFactory {
    type StableId = PollFactoryStableId;
    fn stable_id(&self) -> Self::StableId {
        self.stable_id
    }
}

pub fn unsafe_update_factory_state(data: &mut PlutusData, last_poll_epoch: ProtocolEpoch) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(0, PlutusData::new_integer(last_poll_epoch.into()))
}

pub enum PollFactoryAction {
    CreatePoll,
    ExecuteProposal,
}

impl IntoPlutusData for PollFactoryAction {
    fn into_pd(self) -> PlutusData {
        match self {
            PollFactoryAction::CreatePoll => PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![])),
            PollFactoryAction::ExecuteProposal => {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![]))
            }
        }
    }
}

pub struct FactoryRedeemer {
    pub successor_ix: usize,
    pub action: PollFactoryAction,
}

impl IntoPlutusData for FactoryRedeemer {
    fn into_pd(self) -> PlutusData {
        let mut cpd = ConstrPlutusData::new(
            PF_REDEEMER_MAPPING.successor_ix as u64,
            vec![(self.successor_ix as u64).into_pd()],
        );
        cpd.set_field(
            PF_REDEEMER_MAPPING.action,
            PlutusData::new_list(vec![self.action.into_pd()]),
        );
        PlutusData::ConstrPlutusData(cpd)
    }
}

pub struct RedeemerPollFactoryMapping {
    successor_ix: usize,
    action: usize,
}

const PF_REDEEMER_MAPPING: RedeemerPollFactoryMapping = RedeemerPollFactoryMapping {
    successor_ix: 0,
    action: 1,
};

pub const WP_FACTORY_EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};

pub const GOV_PROXY_EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};
