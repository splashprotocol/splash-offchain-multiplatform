use crate::{
    deployment::{DaoScriptData, ProtocolValidator},
    routines::inflation::TimedOutputRef,
    util::make_constr_pd_indefinite_arr,
};
use cml_chain::{
    assets::AssetName,
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
    OutputRef,
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

use super::voting_escrow::VotingEscrowConfig;

#[derive(Hash, PartialEq, Eq, Serialize, Deserialize, Clone, Debug)]
pub struct ExtendVotingEscrowOrderBundle<Bearer> {
    pub order: ExtendVotingEscrowOnchainOrder,
    pub output_ref: TimedOutputRef,
    pub bearer: Bearer,
}

impl<Bearer> ExtendVotingEscrowOrderBundle<Bearer> {
    pub fn new(order: ExtendVotingEscrowOnchainOrder, output_ref: TimedOutputRef, bearer: Bearer) -> Self {
        Self {
            order,
            output_ref,
            bearer,
        }
    }
}

impl<Bearer> UniqueOrder for ExtendVotingEscrowOrderBundle<Bearer> {
    type TOrderId = OutputRef;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.output_ref.output_ref
    }
}

impl<Bearer> Weighted for ExtendVotingEscrowOrderBundle<Bearer> {
    fn weight(&self) -> OrderWeight {
        // Older orders first
        OrderWeight::from(u64::MAX - self.output_ref.slot.0)
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub struct ExtendVotingEscrowOnchainOrder {
    pub ve_datum: VotingEscrowConfig,
}

impl<C> TryFromLedger<TransactionOutput, C> for ExtendVotingEscrowOnchainOrder
where
    C: Has<DeployedScriptInfo<{ ProtocolValidator::ExtendVeOrder as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let ve_datum = VotingEscrowConfig::try_from_pd(repr.datum()?.into_pd()?)?;
            return Some(Self { ve_datum });
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
            } => make_constr_pd_indefinite_arr(vec![
                PlutusData::new_integer(BigInteger::from(order_input_ix)),
                PlutusData::new_integer(BigInteger::from(voting_escrow_input_ix)),
                PlutusData::new_integer(BigInteger::from(ve_factory_input_ix)),
            ]),
            ExtendVotingEscrowOrderAction::Refund => {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![]))
            }
        }
    }
}

pub fn make_extend_ve_witness_redeemer(
    order_ref: OutputRef,
    order_action: ExtendVotingEscrowOrderAction,
    (ve_ident_policy_id, ve_ident_name): (PolicyId, AssetName),
    (ve_factory_policy_id, ve_factory_name): (PolicyId, AssetName),
) -> PlutusData {
    let ve_ident_asset_pd = make_constr_pd_indefinite_arr(vec![
        PlutusData::new_bytes(ve_ident_policy_id.to_raw_bytes().to_vec()),
        PlutusData::new_bytes(ve_ident_name.to_raw_bytes().to_vec()),
    ]);
    let ve_factory_asset_pd = make_constr_pd_indefinite_arr(vec![
        PlutusData::new_bytes(ve_factory_policy_id.to_raw_bytes().to_vec()),
        PlutusData::new_bytes(ve_factory_name.to_raw_bytes().to_vec()),
    ]);
    make_constr_pd_indefinite_arr(vec![
        order_ref.into_pd(),
        order_action.into_pd(),
        ve_ident_asset_pd,
        ve_factory_asset_pd,
    ])
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

pub fn compute_extend_ve_witness_validator() -> PlutusV2Script {
    let script_bytes = DaoScriptData::global()
        .extend_voting_escrow_order
        .script_bytes
        .clone();
    let script = PlutusV2Script::new(hex::decode(script_bytes).unwrap());
    let params_pd = uplc::PlutusData::Array(vec![uplc::PlutusData::BoundedBytes(PlutusBytes::from(
        script.hash().to_raw_bytes().to_vec(),
    ))]);
    apply_params_validator(
        params_pd,
        &DaoScriptData::global().extend_voting_escrow_witness.script_bytes,
    )
}
