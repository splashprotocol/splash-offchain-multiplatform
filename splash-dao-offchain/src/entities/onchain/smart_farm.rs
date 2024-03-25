use cml_chain::{
    plutus::{ConstrPlutusData, ExUnits, PlutusData},
    utils::BigInt,
    PolicyId,
};
use cml_crypto::RawBytesEncoding;
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, IntoPlutusData};
use spectrum_offchain::data::{Identifier, Stable};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator;

use crate::{constants::MINT_FARM_AUTH_TOKEN_SCRIPT, routines::inflation::SmartFarmSnapshot};

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Debug, Hash, derive_more::Display)]
pub struct FarmId(pub u64);

impl Identifier for FarmId {
    type For = SmartFarmSnapshot;
}

impl IntoPlutusData for FarmId {
    fn into_pd(self) -> cml_chain::plutus::PlutusData {
        cml_chain::plutus::PlutusData::new_integer(BigInt::from(self.0))
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SmartFarm {
    pub farm_id: FarmId,
}

impl Stable for SmartFarm {
    type StableId = FarmId;
    fn stable_id(&self) -> Self::StableId {
        self.farm_id
    }
}

pub struct Redeemer {
    pub successor_out_ix: u32,
    pub action: Action,
}

impl IntoPlutusData for Redeemer {
    fn into_pd(self) -> PlutusData {
        let mut cpd =
            ConstrPlutusData::new(0, vec![PlutusData::Integer(BigInt::from(self.successor_out_ix))]);
        cpd.set_field(1, self.action.into_pd());
        PlutusData::ConstrPlutusData(cpd)
    }
}

pub enum Action {
    Charge,
    DistributeRewards { perm_manager_input_ix: u32 },
}

impl IntoPlutusData for Action {
    fn into_pd(self) -> cml_chain::plutus::PlutusData {
        match self {
            Action::Charge => PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![])),
            Action::DistributeRewards {
                perm_manager_input_ix,
            } => PlutusData::ConstrPlutusData(ConstrPlutusData::new(
                0,
                vec![PlutusData::Integer(BigInt::from(perm_manager_input_ix))],
            )),
        }
    }
}

pub fn compute_mint_farm_auth_token_policy_id(
    splash_policy: PolicyId,
    factory_auth_policy: PolicyId,
) -> PolicyId {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(uplc_pallas_codec::utils::PlutusBytes::from(
            splash_policy.to_raw_bytes().to_vec(),
        )),
        uplc::PlutusData::BoundedBytes(uplc_pallas_codec::utils::PlutusBytes::from(
            factory_auth_policy.to_raw_bytes().to_vec(),
        )),
    ]);
    apply_params_validator(params_pd, MINT_FARM_AUTH_TOKEN_SCRIPT)
}

pub const FARM_EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};
