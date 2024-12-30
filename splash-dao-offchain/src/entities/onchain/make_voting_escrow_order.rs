use cml_chain::{
    plutus::{ConstrPlutusData, PlutusData, PlutusV2Script},
    transaction::TransactionOutput,
    PolicyId,
};
use cml_crypto::{RawBytesEncoding, ScriptHash};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{DatumExtension, IntoPlutusData},
    transaction::TransactionOutputExtension,
    types::TryFromPData,
};
use spectrum_offchain::{
    backlog::data::{OrderWeight, Weighted},
    domain::{order::UniqueOrder, Has},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::{
    deployment::{test_address, DeployedScriptInfo},
    parametrized_validators::apply_params_validator,
};
use uplc_pallas_codec::utils::PlutusBytes;

use crate::{
    constants::{script_bytes::MAKE_VOTING_ESCROW_ORDER, MAKE_VOTING_ESCROW_ORDER_MIN_LOVELACES},
    deployment::ProtocolValidator,
    routines::inflation::TimedOutputRef,
};

use super::voting_escrow::VotingEscrowConfig;

impl UniqueOrder for MakeVotingEscrowOrder {
    type TOrderId = TimedOutputRef;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.timed_output_ref
    }
}

impl Weighted for MakeVotingEscrowOrder {
    fn weight(&self) -> OrderWeight {
        // Older orders first
        OrderWeight::from(u64::MAX - self.timed_output_ref.slot.0)
    }
}

pub enum MakeVotingEscrowOrderAction {
    Deposit,
    Refund,
}

impl IntoPlutusData for MakeVotingEscrowOrderAction {
    fn into_pd(self) -> cml_chain::plutus::PlutusData {
        match self {
            MakeVotingEscrowOrderAction::Deposit => {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![]))
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
    pub timed_output_ref: TimedOutputRef,
}

impl<C> TryFromLedger<TransactionOutput, C> for MakeVotingEscrowOrder
where
    C: Has<DeployedScriptInfo<{ ProtocolValidator::MakeVeOrder as u8 }>> + Has<TimedOutputRef>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            let timed_output_ref = ctx.select::<TimedOutputRef>();
            if value.coin >= MAKE_VOTING_ESCROW_ORDER_MIN_LOVELACES {
                let ve_datum = VotingEscrowConfig::try_from_pd(repr.datum()?.into_pd()?)?;
                return Some(Self {
                    ve_datum,
                    timed_output_ref,
                });
            }
        }
        None
    }
}

pub fn compute_make_ve_order_validator(
    mint_identifier_policy: PolicyId,
    mint_composition_token_policy: PolicyId,
    ve_script_hash: ScriptHash,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(mint_identifier_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(
            mint_composition_token_policy.to_raw_bytes().to_vec(),
        )),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(ve_script_hash.to_raw_bytes().to_vec())),
    ]);
    apply_params_validator(params_pd, MAKE_VOTING_ESCROW_ORDER)
}
