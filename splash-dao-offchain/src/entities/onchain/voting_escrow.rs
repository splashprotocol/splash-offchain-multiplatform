use std::time::Duration;

use cml_chain::plutus::PlutusV2Script;
use cml_chain::utils::BigInteger;
use cml_chain::{
    plutus::{ConstrPlutusData, PlutusData},
    transaction::TransactionOutput,
    PolicyId,
};
use cml_crypto::{RawBytesEncoding, ScriptHash};
use log::{error, info};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::plutus_data::DatumExtension;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::AssetName;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use uplc_pallas_codec::utils::{Int, PlutusBytes};

use spectrum_cardano_lib::{
    plutus_data::{ConstrPlutusDataExtension, IntoPlutusData, PlutusDataExtension},
    Token,
};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator;

use crate::constants::GT_NAME;
use crate::deployment::{DaoScriptData, ProtocolValidator};
use crate::entities::Snapshot;
use crate::protocol_config::{GTAuthPolicy, MintVEIdentifierPolicy};
use crate::routines::inflation::TimedOutputRef;
use crate::{
    constants::time::MAX_LOCK_TIME_SECONDS,
    time::{NetworkTime, ProtocolEpoch},
};

pub type VotingEscrowSnapshot = Snapshot<VotingEscrow, TimedOutputRef>;

/// Identified by GT Token
#[derive(
    Copy,
    Clone,
    PartialEq,
    Eq,
    Ord,
    PartialOrd,
    Hash,
    Debug,
    Serialize,
    Deserialize,
    derive_more::From,
    derive_more::Display,
)]
/// Each voting_escrow is identified by the name of its identifier NFT.
pub struct VotingEscrowId(pub AssetName);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VotingEscrow {
    pub gov_token_amount: u64,
    pub gt_policy: PolicyId,
    pub gt_auth_name: AssetName,
    pub locked_until: Lock,
    pub ve_identifier_name: AssetName,
    pub owner: Owner,
    pub max_ex_fee: u32,
    pub version: u32,
    pub last_wp_epoch: i32,
    pub last_gp_deadline: i32,
}

impl VotingEscrow {
    pub fn voting_power(&self, current_posix_time: u64) -> u64 {
        match self.locked_until {
            Lock::Def(network_time) => {
                if network_time < current_posix_time {
                    0
                } else {
                    let weighting_power = self.gov_token_amount * (network_time - current_posix_time)
                        / 1000
                        / MAX_LOCK_TIME_SECONDS;
                    info!(
                        "WEIGHTING_POWER: {}, # GT tokens: {}, unlock_time: {}, current_posix_time: {}",
                        weighting_power, self.gov_token_amount, network_time, current_posix_time,
                    );
                    weighting_power
                }
            }
            Lock::Indef(d) => self.gov_token_amount * d.as_secs() / MAX_LOCK_TIME_SECONDS,
        }
    }

    pub fn get_token(&self) -> Token {
        Token(self.gt_policy, self.gt_auth_name)
    }
}

impl Stable for VotingEscrow {
    type StableId = VotingEscrowId;
    fn stable_id(&self) -> Self::StableId {
        VotingEscrowId(self.ve_identifier_name)
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for VotingEscrowSnapshot
where
    C: Has<MintVEIdentifierPolicy>
        + Has<GTAuthPolicy>
        + Has<TimedOutputRef>
        + Has<DeployedScriptInfo<{ ProtocolValidator::VotingEscrow as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            let VotingEscrowConfig {
                locked_until,
                owner,
                max_ex_fee,
                version,
                last_wp_epoch,
                last_gp_deadline,
            } = VotingEscrowConfig::try_from_pd(repr.datum()?.into_pd()?)?;

            match &owner {
                Owner::PubKey(key_bytes) => {
                    if cml_crypto::PublicKey::from_raw_bytes(key_bytes).is_err() {
                        error!("Voting_escrow doesn't contain a valid owner public key!");
                        return None;
                    }
                }
                Owner::Script(_) => (),
            }

            let ve_identifier_policy = ctx.select::<MintVEIdentifierPolicy>().0;
            let mut identifier_token_names = value
                .multiasset
                .iter()
                .find_map(|(policy_id, a)| {
                    if *policy_id == ve_identifier_policy {
                        Some(a)
                    } else {
                        None
                    }
                })?
                .clone();
            assert_eq!(identifier_token_names.len(), 1);
            let (ve_identifier_name_cml, qty) = identifier_token_names.pop_front()?;
            let ve_identifier_name = AssetName::from(ve_identifier_name_cml);
            assert_eq!(qty, 1);
            let gt_policy = ctx.select::<GTAuthPolicy>().0;
            let cml_gt_policy_name =
                cml_chain::assets::AssetName::new(GT_NAME.to_be_bytes().to_vec()).unwrap();

            let gov_token_amount = value.multiasset.get(&gt_policy, &cml_gt_policy_name)?;
            let gt_auth_name = AssetName::from(cml_gt_policy_name);

            let voting_escrow = VotingEscrow {
                gov_token_amount,
                gt_policy,
                gt_auth_name,
                locked_until,
                ve_identifier_name,
                owner,
                max_ex_fee,
                version,
                last_wp_epoch,
                last_gp_deadline,
            };
            let version = ctx.select::<TimedOutputRef>();
            return Some(Snapshot::new(voting_escrow, version));
        }
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, derive_more::Display, Debug, Hash)]
#[display("VotingEscrowConfig owner: {}", owner)]
pub struct VotingEscrowConfig {
    pub locked_until: Lock,
    pub owner: Owner,
    pub max_ex_fee: u32,
    pub version: u32,
    pub last_wp_epoch: i32,
    pub last_gp_deadline: i32,
}

impl IntoPlutusData for VotingEscrowConfig {
    fn into_pd(self) -> PlutusData {
        let locked_until = self.locked_until.into_pd();
        let owner = self.owner.into_pd();
        let max_ex_fee = PlutusData::new_integer(BigInteger::from(self.max_ex_fee));
        let version = PlutusData::new_integer(BigInteger::from(self.version));
        let last_wp_epoch = PlutusData::new_integer(BigInteger::from(self.last_wp_epoch));
        let last_gp_deadline = PlutusData::new_integer(BigInteger::from(self.last_gp_deadline));
        PlutusData::new_constr_plutus_data(ConstrPlutusData::new(
            0,
            vec![
                locked_until,
                owner,
                max_ex_fee,
                version,
                last_wp_epoch,
                last_gp_deadline,
            ],
        ))
    }
}

impl TryFromPData for VotingEscrowConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let locked_until = Lock::try_from_pd(cpd.take_field(0)?)?;
        let owner = Owner::try_from_pd(cpd.take_field(1)?)?;
        let max_ex_fee = cpd.take_field(2)?.into_u64()? as u32;
        let version = cpd.take_field(3)?.into_u64()? as u32;
        let last_wp_epoch = cpd.take_field(4)?.into_i128()? as i32;
        let last_gp_deadline = cpd.take_field(5)?.into_i128()? as i32;

        Some(Self {
            locked_until,
            owner,
            max_ex_fee,
            version,
            last_wp_epoch,
            last_gp_deadline,
        })
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Lock {
    Def(NetworkTime),
    Indef(Duration),
}

impl IntoPlutusData for Lock {
    fn into_pd(self) -> PlutusData {
        match self {
            Lock::Def(n) => PlutusData::ConstrPlutusData(ConstrPlutusData::new(
                0,
                vec![PlutusData::new_integer(n.into())],
            )),
            Lock::Indef(d) => PlutusData::ConstrPlutusData(ConstrPlutusData::new(
                1,
                vec![PlutusData::new_integer(d.as_millis().into())],
            )),
        }
    }
}

impl TryFromPData for Lock {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        if cpd.alternative == 0 {
            let pd = cpd.take_field(0)?;
            let n = pd.into_u64()?;
            return Some(Lock::Def(n));
        } else if cpd.alternative == 1 {
            let pd = cpd.take_field(0)?;
            let millis = pd.clone().into_u64()?;
            return Some(Lock::Indef(Duration::from_millis(millis)));
        }
        None
    }
}

#[derive(Clone, Debug, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum Owner {
    PubKey([u8; 32]),
    Script(ScriptHash),
}

impl std::fmt::Display for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Owner::PubKey(bytes) => {
                write!(f, "Owner::PubKey({})", hex::encode(bytes))
            }
            Owner::Script(script_hash) => {
                write!(f, "Owner::Script({})", script_hash)
            }
        }
    }
}

impl IntoPlutusData for Owner {
    fn into_pd(self) -> PlutusData {
        PlutusData::new_constr_plutus_data(match self {
            Owner::PubKey(vec) => ConstrPlutusData::new(0, vec![PlutusData::new_bytes(vec.to_vec())]),
            Owner::Script(script_hash) => {
                let bytes = script_hash.to_raw_bytes().to_vec();
                ConstrPlutusData::new(1, vec![PlutusData::new_bytes(bytes)])
            }
        })
    }
}

impl TryFromPData for Owner {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        if cpd.alternative == 0 {
            let pd = cpd.take_field(0)?;
            if let Ok(bytes) = pd.clone().into_bytes()?.try_into() {
                return Some(Owner::PubKey(bytes));
            }
        } else if cpd.alternative == 1 {
            let pd = cpd.take_field(0)?;
            if let Ok(script_hash) = ScriptHash::from_raw_bytes(&pd.clone().into_bytes()?) {
                return Some(Owner::Script(script_hash));
            }
        }
        None
    }
}

pub fn unsafe_update_ve_state(data: &mut PlutusData, last_poll_epoch: ProtocolEpoch, new_version: u32) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(3, PlutusData::new_integer(new_version.into()));
    cpd.set_field(4, PlutusData::new_integer(last_poll_epoch.into()))
}
pub enum VotingEscrowAction {
    /// Apply governance action.
    Governance,
    /// Add budget (ADA) to funds execution of Gov actions or increase lock time.
    AddBudgetOrExtend { ve_out_ix: u32 },
    /// Redeem liqudity for voting power.
    Redeem { ve_factory_in_ix: u32 },
}

impl IntoPlutusData for VotingEscrowAction {
    fn into_pd(self) -> PlutusData {
        match self {
            VotingEscrowAction::Governance => PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![])),
            VotingEscrowAction::AddBudgetOrExtend { ve_out_ix } => PlutusData::ConstrPlutusData(
                ConstrPlutusData::new(1, vec![PlutusData::Integer(BigInteger::from(ve_out_ix))]),
            ),
            VotingEscrowAction::Redeem { ve_factory_in_ix } => PlutusData::ConstrPlutusData(
                ConstrPlutusData::new(2, vec![PlutusData::Integer(BigInteger::from(ve_factory_in_ix))]),
            ),
        }
    }
}

pub struct VotingEscrowAuthorizedAction {
    pub action: VotingEscrowAction,
    /// Hash of the script authorized to witness the TX.
    pub witness: ScriptHash,
    /// Version to which the action can be applied.
    pub version: u32,
    /// Proof that the owner did authorize the action with the specified version of the voting escrow.
    pub signature: Vec<u8>,
}

pub struct RedeemerVotingEscrowAuthorizedActionMapping {
    pub action: usize,
    /// Hash of the script authorized to witness the TX.
    pub witness: usize,
    /// Version to which the action can be applied.
    pub version: usize,
    /// Proof that the owner did authorize the action with the specified version of the voting escrow.
    pub signature: usize,
}

const VEAA_REDEEMER_MAPPING: RedeemerVotingEscrowAuthorizedActionMapping =
    RedeemerVotingEscrowAuthorizedActionMapping {
        action: 0,
        witness: 1,
        version: 2,
        signature: 3,
    };

impl IntoPlutusData for VotingEscrowAuthorizedAction {
    fn into_pd(self) -> PlutusData {
        let cpd = ConstrPlutusData::new(
            VEAA_REDEEMER_MAPPING.action as u64,
            vec![
                self.action.into_pd(),
                PlutusData::new_bytes(self.witness.to_raw_bytes().to_vec()),
                PlutusData::new_integer(BigInteger::from(self.version)),
                PlutusData::new_bytes(self.signature),
            ],
        );
        PlutusData::ConstrPlutusData(cpd)
    }
}

pub enum MintAction {
    MintPower {
        binder: u32,
        ve_in_ix: u32,
        proposal_in_ix: u32,
    },
    Burn,
}

impl IntoPlutusData for MintAction {
    fn into_pd(self) -> PlutusData {
        match self {
            MintAction::MintPower {
                binder,
                ve_in_ix,
                proposal_in_ix,
            } => PlutusData::ConstrPlutusData(ConstrPlutusData::new(
                0,
                vec![
                    PlutusData::Integer(BigInteger::from(binder)),
                    PlutusData::Integer(BigInteger::from(ve_in_ix)),
                    PlutusData::Integer(BigInteger::from(proposal_in_ix)),
                ],
            )),
            MintAction::Burn => PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![])),
        }
    }
}

pub const MIN_ADA_IN_BOX: u64 = 1_000_000;

pub fn compute_mint_weighting_power_validator(
    zeroth_epoch_start: u64,
    proposal_auth_policy: PolicyId,
    gt_policy: PolicyId,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BigInt(uplc::BigInt::Int(Int::from(zeroth_epoch_start as i64))),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(proposal_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(gt_policy.to_raw_bytes().to_vec())),
    ]);
    apply_params_validator(
        params_pd,
        &DaoScriptData::global().mint_weighting_power.script_bytes,
    )
}

pub fn compute_voting_escrow_validator(
    ve_identifier_policy: PolicyId,
    ve_factory_auth_policy: PolicyId,
    ve_composition_policy: PolicyId,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(ve_identifier_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(ve_factory_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(ve_composition_policy.to_raw_bytes().to_vec())),
    ]);
    apply_params_validator(params_pd, &DaoScriptData::global().voting_escrow.script_bytes)
}

pub fn compute_mint_governance_power_validator(
    proposal_auth_policy: PolicyId,
    gt_policy: PolicyId,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(proposal_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(gt_policy.to_raw_bytes().to_vec())),
    ]);
    apply_params_validator(
        params_pd,
        &DaoScriptData::global().mint_governance_power.script_bytes,
    )
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, time::Duration};

    use cbor_event::de::Deserializer;
    use cml_chain::{plutus::PlutusData, Deserialize};
    use spectrum_cardano_lib::types::TryFromPData;

    use crate::entities::onchain::voting_escrow::{Lock, VotingEscrowConfig};

    #[test]
    fn test_ve_datum_deserialization() {
        // Hex-encoded datum from preprod deployment TX
        let bytes = hex::decode("d8799fd8799f01ffd8799f5820d129974b472a9ca1148791369969572e0db24075649211b60472e52a3fb3401aff01010101ff").unwrap();
        let mut raw = Deserializer::from(Cursor::new(bytes));

        let data = PlutusData::deserialize(&mut raw).unwrap();
        assert!(VotingEscrowConfig::try_from_pd(data).is_some());
    }

    #[test]
    fn as_json() {
        println!("{}", serde_json::to_string_pretty(&Lock::Def(1000)).unwrap());
        println!(
            "{}",
            serde_json::to_string_pretty(&Lock::Indef(Duration::from_secs(123))).unwrap()
        );
    }
}
