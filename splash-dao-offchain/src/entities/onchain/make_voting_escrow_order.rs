use cml_chain::{
    plutus::{ConstrPlutusData, PlutusData, PlutusV2Script},
    transaction::TransactionOutput,
    utils::BigInteger,
    PolicyId,
};
use cml_crypto::{RawBytesEncoding, ScriptHash};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{DatumExtension, IntoPlutusData},
    transaction::TransactionOutputExtension,
    types::TryFromPData,
    OutputRef,
};
use spectrum_offchain::{
    backlog::data::{OrderWeight, Weighted},
    domain::{order::UniqueOrder, Has},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::{
    deployment::{test_address, DeployedScriptInfo},
    parametrized_validators::apply_params_validator_plutus_v2,
};
use uplc_pallas_primitives::{BoundedBytes, MaybeIndefArray};

use crate::{
    constants::MAKE_VOTING_ESCROW_ORDER_MIN_LOVELACES,
    deployment::{DaoScriptData, ProtocolValidator},
    routines::inflation::TimedOutputRef,
};

use super::voting_escrow::{VotingEscrowConfig, VotingEscrowId};

#[derive(Hash, PartialEq, Eq, Serialize, Deserialize, Clone, Debug)]
pub struct MakeVotingEscrowOrderBundle<Bearer> {
    pub order: MakeVotingEscrowOrder,
    pub output_ref: TimedOutputRef,
    pub bearer: Bearer,
}

impl<Bearer> MakeVotingEscrowOrderBundle<Bearer> {
    pub fn new(order: MakeVotingEscrowOrder, output_ref: TimedOutputRef, bearer: Bearer) -> Self {
        Self {
            order,
            output_ref,
            bearer,
        }
    }
}

impl<Bearer> UniqueOrder for MakeVotingEscrowOrderBundle<Bearer> {
    type TOrderId = OutputRef;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.output_ref.output_ref
    }
}

impl<Bearer> Weighted for MakeVotingEscrowOrderBundle<Bearer> {
    fn weight(&self) -> OrderWeight {
        // Older orders first
        OrderWeight::from(u64::MAX - self.output_ref.slot.0)
    }
}

pub enum MakeVotingEscrowOrderAction {
    Deposit { ve_factory_input_ix: u32 },
    Refund,
}

impl IntoPlutusData for MakeVotingEscrowOrderAction {
    fn into_pd(self) -> cml_chain::plutus::PlutusData {
        match self {
            MakeVotingEscrowOrderAction::Deposit { ve_factory_input_ix } => {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(
                    0,
                    vec![PlutusData::new_integer(BigInteger::from(ve_factory_input_ix))],
                ))
            }

            MakeVotingEscrowOrderAction::Refund => {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![]))
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub struct MakeVotingEscrowOrder {
    pub ve_datum: VotingEscrowConfig,
}

impl<C> TryFromLedger<TransactionOutput, C> for MakeVotingEscrowOrder
where
    C: Has<DeployedScriptInfo<{ ProtocolValidator::MakeVeOrder as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            if value.coin >= MAKE_VOTING_ESCROW_ORDER_MIN_LOVELACES {
                let ve_datum = VotingEscrowConfig::try_from_pd(repr.datum()?.into_pd()?)?;
                return Some(Self { ve_datum });
            }
        }
        None
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub enum MVEStatus {
    Unspent,
    Refunded,
    SpentToFormVotingEscrow(VotingEscrowId),
}

pub fn compute_make_ve_order_validator(mint_composition_token_policy: PolicyId) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(MaybeIndefArray::Indef(vec![uplc::PlutusData::BoundedBytes(
        BoundedBytes::from(mint_composition_token_policy.to_raw_bytes().to_vec()),
    )]));
    apply_params_validator_plutus_v2(
        params_pd,
        &DaoScriptData::global().make_voting_escrow_order.script_bytes,
    )
}
