use crate::deployment::{DaoScriptData, ProtocolValidator};
use crate::entities::onchain::weighting_poll::Farm;
use crate::entities::Snapshot;
use crate::protocol_config::PermManagerAuthPolicy;
use crate::routines::TimedOutputRef;
use cml_chain::plutus::PlutusV2Script;
use cml_chain::transaction::TransactionOutput;
use cml_chain::utils::BigInteger;
use cml_chain::{
    plutus::{ConstrPlutusData, PlutusData},
    PolicyId,
};
use cml_core::serialization::ToBytes;
use cml_crypto::RawBytesEncoding;
use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::plutus_data::{
    make_constr_pd_indefinite_arr, ConstrPlutusDataExtension, DatumExtension, IntoPlutusData,
    PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{AssetName, Token};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::PoolId;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator_plutus_v2;
use uplc_pallas_primitives::{BoundedBytes, MaybeIndefArray};

pub type SmartFarmSnapshot = Snapshot<SmartFarm, TimedOutputRef>;

#[derive(
    Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Debug, Hash, derive_more::Display, Serialize, Deserialize,
)]
pub struct FarmId(pub AssetName);

impl FarmId {
    pub fn random() -> Self {
        let random_string: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(6)
            .map(char::from)
            .collect();

        let tn = AssetName::from_utf8(random_string);
        FarmId(tn)
    }
}

impl From<FarmId> for Vec<u8> {
    fn from(FarmId(value): FarmId) -> Self {
        value.as_bytes().to_bytes()
    }
}

impl IntoPlutusData for FarmId {
    fn into_pd(self) -> PlutusData {
        PlutusData::new_bytes(cml_chain::assets::AssetName::from(self.0).inner)
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

pub struct SmartFarmConfig {
    pub perm_manager_auth_policy: PolicyId,
    pub pool_id: PoolId,
}

impl TryFromPData for SmartFarmConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            perm_manager_auth_policy: PolicyId::try_from_pd(cpd.take_field(0)?)?,
            pool_id: Token::try_from_pd(cpd.take_field(1)?)?.into(),
        })
    }
}

impl IntoPlutusData for SmartFarmConfig {
    fn into_pd(self) -> PlutusData {
        let pool_id_pd = make_constr_pd_indefinite_arr(vec![
            PlutusData::new_bytes(self.pool_id.0 .0.to_raw_bytes().to_vec()),
            PlutusData::new_bytes(self.pool_id.0 .1.as_bytes().to_vec()),
        ]);
        make_constr_pd_indefinite_arr(vec![
            PlutusData::new_bytes(self.perm_manager_auth_policy.to_raw_bytes().to_vec()),
            pool_id_pd,
        ])
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SmartFarm {
    pub farm_id: FarmId,
    pub pool_id: PoolId,
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

pub struct Redeemer {
    pub successor_out_ix: u32,
    pub action: Action,
}

impl IntoPlutusData for Redeemer {
    fn into_pd(self) -> PlutusData {
        let cpd = ConstrPlutusData::new(
            0,
            vec![
                PlutusData::Integer(BigInteger::from(self.successor_out_ix)),
                self.action.into_pd(),
            ],
        );

        // This wrapping is needed since `smart_farm` is a multivalidator with `mint_farm_auth_token`.
        PlutusData::new_constr_plutus_data(ConstrPlutusData::new(1, vec![PlutusData::ConstrPlutusData(cpd)]))
    }
}

pub enum Action {
    Charge,
    DistributeRewards { perm_manager_input_ix: u32 },
}

impl IntoPlutusData for Action {
    fn into_pd(self) -> PlutusData {
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
        + Has<TimedOutputRef>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        let addr = repr.address();
        if test_address(addr, ctx) {
            let conf = SmartFarmConfig::try_from_pd(repr.datum()?.into_pd()?)?;
            if ctx.select::<PermManagerAuthPolicy>().0 == conf.perm_manager_auth_policy {
                let value = repr.value();
                let farm_auth_policy = ctx
                    .select::<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>()
                    .script_hash;
                for (policy_id, by_names) in value.multiasset.iter() {
                    if *policy_id == farm_auth_policy && by_names.len() == 1 {
                        let (farm_name, quantity) = by_names.front()?;
                        if *quantity == 1 {
                            let smart_farm = SmartFarm {
                                farm_id: FarmId(spectrum_cardano_lib::AssetName::from(farm_name.clone())),
                                pool_id: conf.pool_id,
                            };
                            let version = ctx.select::<TimedOutputRef>();
                            return Some(Snapshot::new(smart_farm, version));
                        }
                    }
                }
            }
        }
        None
    }
}

pub enum MintAction {
    MintAuthToken { factory_in_ix: u32 },
    BurnAuthToken,
}

impl IntoPlutusData for MintAction {
    fn into_pd(self) -> PlutusData {
        match self {
            MintAction::MintAuthToken { factory_in_ix } => PlutusData::ConstrPlutusData(
                ConstrPlutusData::new(0, vec![PlutusData::Integer(BigInteger::from(factory_in_ix))]),
            ),
            MintAction::BurnAuthToken => PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![])),
        }
    }
}

pub fn compute_mint_farm_auth_token_validator(
    splash_policy: PolicyId,
    factory_auth_policy: PolicyId,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(MaybeIndefArray::Indef(vec![
        uplc::PlutusData::BoundedBytes(BoundedBytes::from(splash_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(BoundedBytes::from(factory_auth_policy.to_raw_bytes().to_vec())),
    ]));
    apply_params_validator_plutus_v2(
        params_pd,
        &DaoScriptData::global().mint_farm_auth_token.script_bytes,
    )
}
