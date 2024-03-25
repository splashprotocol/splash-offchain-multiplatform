use std::fmt::Formatter;

use cml_chain::{plutus::ExUnits, PolicyId};
use cml_crypto::RawBytesEncoding;
use derive_more::From;
use spectrum_cardano_lib::Token;
use spectrum_offchain::data::{Identifier, Stable};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator;

use crate::{constants::PERM_MANAGER_SCRIPT, routines::inflation::PermManagerSnapshot};

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd, From)]
pub struct PermManagerId(Token);

impl Identifier for PermManagerId {
    type For = PermManagerSnapshot;
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PermManager {
    pub stable_id: PermManagerStableId,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PermManagerStableId {
    edao_msig_policy: PolicyId,
    /// An NFT mint once on setup.
    perm_manager_auth_policy: PolicyId,
}

impl std::fmt::Display for PermManagerStableId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "PermManagerStableId: edao_msig_policy: {}, perm_manager_auth_policy: {}",
            self.edao_msig_policy, self.perm_manager_auth_policy
        ))
    }
}

impl Stable for PermManager {
    type StableId = PermManagerStableId;
    fn stable_id(&self) -> Self::StableId {
        self.stable_id
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
