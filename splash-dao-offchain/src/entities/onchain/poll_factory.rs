use std::fmt::Formatter;

use cml_chain::plutus::{ConstrPlutusData, ExUnits, PlutusData};

use cml_chain::PolicyId;
use cml_crypto::{RawBytesEncoding, ScriptHash};
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::{TaggedAmount, Token};
use spectrum_offchain::data::{Identifier, Stable};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator;
use uplc_pallas_codec::utils::PlutusBytes;

use crate::assets::Splash;
use crate::constants::WP_FACTORY_SCRIPT;
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
    pub fn next_weighting_poll(
        mut self,
        farm_auth_policy: PolicyId,
        emission_rate: TaggedAmount<Splash>,
    ) -> (PollFactory, WeightingPoll) {
        let poll_epoch = self.last_poll_epoch + 1;
        let stable_id = WeightingPollStableId {
            auth_policy: self.stable_id.wp_auth_policy,
            farm_auth_policy,
        };
        let next_poll = WeightingPoll {
            epoch: poll_epoch,
            distribution: self.active_farms.iter().map(|farm| (*farm, 0u64)).collect(),
            stable_id,
            emission_rate,
            weighting_power: None,
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
    fn is_quasi_permanent(&self) -> bool {
        true
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

pub fn compute_wp_factory_script_hash(
    wp_auth_policy: PolicyId,
    gov_witness_script_hash: ScriptHash,
) -> ScriptHash {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(wp_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(gov_witness_script_hash.to_raw_bytes().to_vec())),
    ]);
    apply_params_validator(params_pd, WP_FACTORY_SCRIPT)
}
