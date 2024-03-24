use std::{fmt::Formatter, time::Duration};

use cml_chain::{
    plutus::{ConstrPlutusData, ExUnits, PlutusData},
    transaction::TransactionOutput,
    utils::BigInt,
    PolicyId,
};
use cml_core::serialization::FromBytes;
use cml_crypto::{chain_crypto::Signature, RawBytesEncoding, ScriptHash};
use spectrum_cardano_lib::{
    plutus_data::{ConstrPlutusDataExtension, IntoPlutusData, PlutusDataExtension},
    Token,
};
use spectrum_offchain::{
    data::{EntitySnapshot, Identifier, Stable},
    ledger::IntoLedger,
};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator;
use uplc_pallas_codec::utils::{Int, PlutusBytes};

use crate::{
    constants::{MAX_LOCK_TIME_SECONDS, MINT_WEIGHTING_POWER_SCRIPT, VOTING_ESCROW_SCRIPT},
    routines::inflation::VotingEscrowSnapshot,
    time::{NetworkTime, ProtocolEpoch},
};

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Hash, Debug)]
pub struct VotingEscrowId(Token);

impl Identifier for VotingEscrowId {
    type For = VotingEscrowSnapshot;
}

#[derive(Copy, Clone, Debug)]
pub struct VotingEscrow {
    pub gov_token_amount: u64,
    pub gt_policy: PolicyId,
    pub locked_until: Lock,
    pub stable_id: VotingEscrowStableId,
}

impl VotingEscrow {
    pub fn voting_power(&self, current_posix_time: u64) -> u64 {
        match self.locked_until {
            Lock::Def(network_time) => {
                if network_time < current_posix_time {
                    0
                } else {
                    self.gov_token_amount * (network_time - current_posix_time) / 1000 / MAX_LOCK_TIME_SECONDS
                }
            }
            Lock::Indef(d) => self.gov_token_amount * d.as_secs() / MAX_LOCK_TIME_SECONDS,
        }
    }
}

impl<Ctx> IntoLedger<TransactionOutput, Ctx> for VotingEscrow {
    fn into_ledger(self, ctx: Ctx) -> TransactionOutput {
        todo!()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct VotingEscrowStableId {
    ve_factory_auth_policy: PolicyId,
}

impl std::fmt::Display for VotingEscrowStableId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "VotingEscrowStableId: ve_factory_auth_policy: {}",
            self.ve_factory_auth_policy,
        ))
    }
}

impl Stable for VotingEscrow {
    type StableId = VotingEscrowStableId;
    fn stable_id(&self) -> Self::StableId {
        self.stable_id
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Lock {
    Def(NetworkTime),
    Indef(Duration),
}

pub fn unsafe_update_ve_state(data: &mut PlutusData, last_poll_epoch: ProtocolEpoch) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(4, PlutusData::new_integer(last_poll_epoch.into()))
}
pub enum VotingEscrowAction {
    /// Apply governance action.
    Governance,
    /// Add budget (ADA) to funds execution of Gov actions or increase lock time.
    AddBudgetOrExtend,
    /// Redeem liqudity for voting power.
    Redeem { ve_factory_in_ix: u32 },
}

impl IntoPlutusData for VotingEscrowAction {
    fn into_pd(self) -> PlutusData {
        match self {
            VotingEscrowAction::Governance => PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![])),
            VotingEscrowAction::AddBudgetOrExtend => {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![]))
            }
            VotingEscrowAction::Redeem { ve_factory_in_ix } => PlutusData::ConstrPlutusData(
                ConstrPlutusData::new(2, vec![PlutusData::Integer(BigInt::from(ve_factory_in_ix))]),
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
        let mut cpd = ConstrPlutusData::new(VEAA_REDEEMER_MAPPING.action as u64, vec![self.action.into_pd()]);
        cpd.set_field(
            VEAA_REDEEMER_MAPPING.witness,
            PlutusData::new_bytes(self.witness.to_raw_bytes().to_vec()),
        );
        cpd.set_field(
            VEAA_REDEEMER_MAPPING.version,
            PlutusData::new_integer(BigInt::from(self.version)),
        );
        cpd.set_field(
            VEAA_REDEEMER_MAPPING.signature,
            PlutusData::new_bytes(self.signature),
        );
        PlutusData::ConstrPlutusData(cpd)
    }
}

pub const VOTING_ESCROW_EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};

pub const WEIGHTING_POWER_EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};

pub const ORDER_WITNESS_EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};

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
                    PlutusData::Integer(BigInt::from(binder)),
                    PlutusData::Integer(BigInt::from(ve_in_ix)),
                    PlutusData::Integer(BigInt::from(proposal_in_ix)),
                ],
            )),
            MintAction::Burn => PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![])),
        }
    }
}

pub fn compute_mint_weighting_power_policy_id(
    zeroth_epoch_start: u64,
    proposal_auth_policy: PolicyId,
    gt_policy: PolicyId,
) -> PolicyId {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BigInt(uplc::BigInt::Int(Int::from(zeroth_epoch_start as i64))),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(proposal_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(gt_policy.to_raw_bytes().to_vec())),
    ]);
    apply_params_validator(params_pd, MINT_WEIGHTING_POWER_SCRIPT)
}

pub fn compute_voting_escrow_policy_id(ve_factory_auth_policy: PolicyId) -> PolicyId {
    let params_pd = uplc::PlutusData::Array(vec![uplc::PlutusData::BoundedBytes(PlutusBytes::from(
        ve_factory_auth_policy.to_raw_bytes().to_vec(),
    ))]);
    apply_params_validator(params_pd, VOTING_ESCROW_SCRIPT)
}
