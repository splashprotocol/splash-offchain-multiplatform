use crate::{
    deployment::{DaoScriptData, ProtocolValidator},
    routines::inflation::TimedOutputRef,
};
use cml_chain::{
    plutus::{ConstrPlutusData, PlutusData, PlutusV2Script},
    transaction::TransactionOutput,
    utils::BigInteger,
    PolicyId,
};
use cml_crypto::RawBytesEncoding;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{DatumExtension, IntoPlutusData},
    types::TryFromPData,
};
use spectrum_offchain::{domain::Has, ledger::TryFromLedger};
use spectrum_offchain_cardano::{
    deployment::{test_address, DeployedScriptInfo},
    parametrized_validators::apply_params_validator,
};
use uplc_pallas_codec::utils::PlutusBytes;

use super::voting_escrow::VotingEscrowConfig;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub struct ExtendVotingEscrowOrder {
    pub ve_datum: VotingEscrowConfig,
    pub timed_output_ref: TimedOutputRef,
}

impl<C> TryFromLedger<TransactionOutput, C> for ExtendVotingEscrowOrder
where
    C: Has<DeployedScriptInfo<{ ProtocolValidator::ExtendVeOrder as u8 }>> + Has<TimedOutputRef>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let timed_output_ref = ctx.select::<TimedOutputRef>();
            let ve_datum = VotingEscrowConfig::try_from_pd(repr.datum()?.into_pd()?)?;
            return Some(Self {
                ve_datum,
                timed_output_ref,
            });
        }
        None
    }
}

pub enum ExtendVotingEscrowOrderAction {
    Extend {
        order_input_ix: u32,
        voting_escrow_input_ix: u32,
        ve_factory_input_ix: u32,
    },
    Refund,
}

impl IntoPlutusData for ExtendVotingEscrowOrderAction {
    fn into_pd(self) -> PlutusData {
        match self {
            ExtendVotingEscrowOrderAction::Extend {
                order_input_ix,
                voting_escrow_input_ix,
                ve_factory_input_ix,
            } => PlutusData::ConstrPlutusData(ConstrPlutusData::new(
                0,
                vec![
                    PlutusData::new_integer(BigInteger::from(order_input_ix)),
                    PlutusData::new_integer(BigInteger::from(voting_escrow_input_ix)),
                    PlutusData::new_integer(BigInteger::from(ve_factory_input_ix)),
                ],
            )),
            ExtendVotingEscrowOrderAction::Refund => {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![]))
            }
        }
    }
}

pub fn compute_extend_ve_order_validator(mint_composition_token_policy: PolicyId) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(vec![uplc::PlutusData::BoundedBytes(PlutusBytes::from(
        mint_composition_token_policy.to_raw_bytes().to_vec(),
    ))]);
    apply_params_validator(
        params_pd,
        &DaoScriptData::global().extend_voting_escrow_order.script_bytes,
    )
}
