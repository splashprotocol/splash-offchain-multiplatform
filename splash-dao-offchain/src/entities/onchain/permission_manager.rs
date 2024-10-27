use cml_chain::{
    plutus::{ConstrPlutusData, ExUnits, PlutusData, PlutusV2Script},
    transaction::TransactionOutput,
    PolicyId,
};
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
use derive_more::From;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension},
    transaction::TransactionOutputExtension,
    types::TryFromPData,
    AssetName, OutputRef, Token,
};
use spectrum_offchain::{
    data::{Has, HasIdentifier, Identifier, Stable},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::{
    deployment::{test_address, DeployedScriptInfo},
    parametrized_validators::apply_params_validator,
};

use crate::{
    constants::{script_bytes::PERM_MANAGER_SCRIPT, DEFAULT_AUTH_TOKEN_NAME},
    deployment::ProtocolValidator,
    entities::Snapshot,
    protocol_config::PermManagerAuthPolicy,
};

use super::smart_farm::FarmId;

#[derive(
    Copy, Clone, PartialEq, Eq, Ord, PartialOrd, From, Serialize, Deserialize, derive_more::Display, Hash,
)]
pub struct PermManagerId;

impl Identifier for PermManagerId {
    type For = PermManagerSnapshot;
}

pub type PermManagerSnapshot = Snapshot<PermManager, OutputRef>;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PermManager {
    pub datum: PermManagerDatum,
}

impl HasIdentifier for PermManagerSnapshot {
    type Id = PermManagerId;

    fn identifier(&self) -> Self::Id {
        PermManagerId
    }
}

impl Stable for PermManager {
    type StableId = PermManagerId;
    fn stable_id(&self) -> Self::StableId {
        PermManagerId
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for PermManagerSnapshot
where
    C: Has<PermManagerAuthPolicy>
        + Has<OutputRef>
        + Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let perm_manager_auth_policy = ctx.select::<PermManagerAuthPolicy>().0;
            let auth_token_cml_asset_name =
                cml_chain::assets::AssetName::new(DEFAULT_AUTH_TOKEN_NAME.to_be_bytes().to_vec()).unwrap();
            let auth_token_qty = repr
                .value()
                .multiasset
                .get(&perm_manager_auth_policy, &auth_token_cml_asset_name)?;
            assert_eq!(auth_token_qty, 1);
            let datum = repr.datum()?;
            let perm_manager_datum = datum
                .into_pd()
                .map(|pd| PermManagerDatum::try_from_pd(pd).unwrap())?;
            let output_ref = ctx.select::<OutputRef>();
            let perm_manager = PermManager {
                datum: perm_manager_datum,
            };

            return Some(Snapshot::new(perm_manager, output_ref));
        }
        None
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PermManagerDatum {
    pub authorized_executors: Vec<Ed25519KeyHash>,
    pub suspended_farms: Vec<FarmId>,
}

impl IntoPlutusData for PermManagerDatum {
    fn into_pd(self) -> PlutusData {
        let authorized_executors_vec: Vec<PlutusData> = self
            .authorized_executors
            .into_iter()
            .map(|key_hash| PlutusData::new_bytes(key_hash.to_raw_bytes().to_vec()))
            .collect();
        let suspended_farms_vec: Vec<PlutusData> = self
            .suspended_farms
            .into_iter()
            .map(|farm_id| {
                let farm_name = cml_chain::assets::AssetName::from(farm_id.0);
                PlutusData::new_bytes(farm_name.to_raw_bytes().to_vec())
            })
            .collect();
        let cpd = ConstrPlutusData::new(
            0,
            vec![
                PlutusData::new_list(authorized_executors_vec),
                PlutusData::new_list(suspended_farms_vec),
            ],
        );
        PlutusData::ConstrPlutusData(cpd)
    }
}

impl TryFromPData for PermManagerDatum {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let authorized_executors = cpd.take_field(0)?.into_vec_pd(|pd| {
            pd.into_bytes()
                .map(|bytes| Ed25519KeyHash::from_raw_bytes(&bytes).unwrap())
        })?;
        let suspended_farms = cpd.take_field(1)?.into_vec_pd(FarmId::try_from_pd)?;
        Some(Self {
            authorized_executors,
            suspended_farms,
        })
    }
}

pub fn compute_perm_manager_validator(
    edao_msig_policy: PolicyId,
    perm_manager_auth_policy: PolicyId,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(uplc_pallas_codec::utils::PlutusBytes::from(
            edao_msig_policy.to_raw_bytes().to_vec(),
        )),
        uplc::PlutusData::BoundedBytes(uplc_pallas_codec::utils::PlutusBytes::from(
            perm_manager_auth_policy.to_raw_bytes().to_vec(),
        )),
    ]);
    apply_params_validator(params_pd, PERM_MANAGER_SCRIPT)
}

pub const PERM_MANAGER_EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};
