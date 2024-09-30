use cml_chain::certs::Credential;
use cml_chain::plutus::PlutusV2Script;
use cml_chain::transaction::TransactionOutput;
use cml_chain::utils::BigInteger;
use cml_chain::{
    plutus::{ConstrPlutusData, ExUnits, PlutusData},
    PolicyId,
};
use cml_crypto::RawBytesEncoding;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{AssetName, OutputRef};
use spectrum_offchain::data::{Has, HasIdentifier, Identifier, Stable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator;

use crate::deployment::ProtocolValidator;
use crate::entities::Snapshot;
use crate::protocol_config::PermManagerAuthPolicy;

pub type SmartFarmSnapshot = Snapshot<SmartFarm, OutputRef>;

#[derive(
    Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Debug, Hash, derive_more::Display, Serialize, Deserialize,
)]
pub struct FarmId(pub AssetName);

impl Identifier for FarmId {
    type For = SmartFarmSnapshot;
}

impl IntoPlutusData for FarmId {
    fn into_pd(self) -> cml_chain::plutus::PlutusData {
        cml_chain::plutus::PlutusData::new_bytes(cml_chain::assets::AssetName::from(self.0).inner)
    }
}

impl TryFromPData for FarmId {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        if let PlutusData::Bytes { bytes, .. } = data {
            return Some(FarmId(AssetName::try_from(bytes).ok()?));
        }
        None
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SmartFarm {
    pub farm_id: FarmId,
}

impl Stable for SmartFarm {
    type StableId = FarmId;
    fn stable_id(&self) -> Self::StableId {
        self.farm_id
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl HasIdentifier for SmartFarmSnapshot {
    type Id = FarmId;

    fn identifier(&self) -> Self::Id {
        self.0.farm_id
    }
}

pub struct Redeemer {
    pub successor_out_ix: u32,
    pub action: Action,
}

impl IntoPlutusData for Redeemer {
    fn into_pd(self) -> PlutusData {
        let mut cpd = ConstrPlutusData::new(
            0,
            vec![PlutusData::Integer(BigInteger::from(self.successor_out_ix))],
        );
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
                1,
                vec![PlutusData::Integer(BigInteger::from(perm_manager_input_ix))],
            )),
        }
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for SmartFarmSnapshot
where
    C: Has<PermManagerAuthPolicy>
        + Has<OutputRef>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        let addr = repr.address();
        if test_address(addr, ctx) {
            if let Ok(auth_policy) = PolicyId::from_raw_bytes(&repr.datum()?.into_pd()?.into_bytes()?) {
                if ctx.select::<PermManagerAuthPolicy>().0 == auth_policy {
                    let cred = addr.payment_cred().unwrap();
                    if let Credential::Script { hash, .. } = cred {
                        let smart_farm = SmartFarm {
                            farm_id: FarmId(AssetName::try_from(hash.to_raw_bytes().to_vec()).unwrap()),
                        };
                        let output_ref = ctx.select::<OutputRef>();
                        return Some(Snapshot::new(smart_farm, output_ref));
                    }
                }
            }
        }
        None
    }
}

pub fn compute_mint_farm_auth_token_validator(
    script: &str,
    splash_policy: PolicyId,
    factory_auth_policy: PolicyId,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(uplc_pallas_codec::utils::PlutusBytes::from(
            splash_policy.to_raw_bytes().to_vec(),
        )),
        uplc::PlutusData::BoundedBytes(uplc_pallas_codec::utils::PlutusBytes::from(
            factory_auth_policy.to_raw_bytes().to_vec(),
        )),
    ]);
    apply_params_validator(params_pd, script)
}

pub const FARM_EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};
