use cml_chain::{plutus::ExUnits, transaction::TransactionOutput, PolicyId};
use cml_crypto::RawBytesEncoding;
use derive_more::From;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension},
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
    constants::PERM_MANAGER_SCRIPT,
    deployment::ProtocolValidator,
    entities::Snapshot,
    protocol_config::{PermManagerAuthName, PermManagerAuthPolicy},
};

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd, From, Serialize, Deserialize)]
pub struct PermManagerId;

impl Identifier for PermManagerId {
    type For = PermManagerSnapshot;
}

pub type PermManagerSnapshot = Snapshot<PermManager, OutputRef>;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PermManager {
    pub perm_manager_auth_policy: PolicyId,
    pub auth_token_asset_name: AssetName,
}

impl PermManager {
    pub fn get_token(&self) -> Token {
        Token(self.perm_manager_auth_policy, self.auth_token_asset_name)
    }
}

impl HasIdentifier for PermManagerSnapshot {
    type Id = PermManagerId;

    fn identifier(&self) -> Self::Id {
        PermManagerId
    }
}

impl Stable for PermManager {
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        self.perm_manager_auth_policy
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for PermManagerSnapshot
where
    C: Has<PermManagerAuthPolicy>
        + Has<PermManagerAuthName>
        + Has<OutputRef>
        + Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let perm_manager_auth_policy = ctx.select::<PermManagerAuthPolicy>().0;
            let auth_token_cml_asset_name = ctx.select::<PermManagerAuthName>().0;
            let auth_token_qty = repr
                .value()
                .multiasset
                .get(&perm_manager_auth_policy, &auth_token_cml_asset_name)?;
            assert_eq!(auth_token_qty, 1);
            let output_ref = ctx.select::<OutputRef>();
            let perm_manager = PermManager {
                perm_manager_auth_policy,
                auth_token_asset_name: AssetName::from(auth_token_cml_asset_name),
            };

            return Some(Snapshot::new(perm_manager, output_ref));
        }
        None
    }
}

pub fn compute_perm_manager_policy_id(
    edao_msig_policy: PolicyId,
    perm_manager_auth_policy: PolicyId,
) -> PolicyId {
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
