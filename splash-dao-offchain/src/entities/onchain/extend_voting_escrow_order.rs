use crate::{
    constants::EXTEND_VOTING_ESCROW_ORDER_MIN_LOVELACES,
    deployment::{DaoScriptData, ProtocolValidator},
    routines::TimedOutputRef,
};
use cml_chain::{
    assets::AssetName,
    auxdata::Metadata,
    plutus::{ConstrPlutusData, PlutusData, PlutusV2Script},
    transaction::TransactionOutput,
    utils::BigInteger,
    PolicyId,
};
use cml_crypto::RawBytesEncoding;
use log::error;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{
        make_constr_pd_indefinite_arr, ConstrPlutusDataExtension, DatumExtension, IntoPlutusData,
        PlutusDataExtension,
    },
    transaction::TransactionOutputExtension,
    types::TryFromPData,
    OutputRef,
};
use spectrum_offchain::{
    backlog::data::{OrderWeight, Weighted},
    domain::{order::UniqueOrder, Has, Stable},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::{
    deployment::{test_address, DeployedScriptInfo},
    parametrized_validators::apply_params_validator_plutus_v2,
};
use uplc_pallas_primitives::{BoundedBytes, MaybeIndefArray};

use super::{
    get_proxy_order_metadata,
    voting_escrow::{Owner, VotingEscrowConfig},
    ProxyOrderMetadata,
};

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
    pub datum: ExtendVotingEscrowOrderState,
    pub metadata: ProxyOrderMetadata,
}

impl<C> TryFromLedger<TransactionOutput, C> for ExtendVotingEscrowOnchainOrder
where
    C: Has<DeployedScriptInfo<{ ProtocolValidator::ExtendVeOrder as u8 }>> + Has<Option<Metadata>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            if value.coin >= EXTEND_VOTING_ESCROW_ORDER_MIN_LOVELACES {
                let datum = ExtendVotingEscrowOrderState::try_from_pd(repr.datum()?.into_pd()?)?;
                let tx_metadata = ctx.select::<Option<Metadata>>()?;
                let metadata = get_proxy_order_metadata(tx_metadata)?;
                return Some(Self { datum, metadata });
            }
        }
        None
    }
}

impl Stable for ExtendVotingEscrowOnchainOrder {
    type StableId = Owner;

    fn stable_id(&self) -> Self::StableId {
        self.datum.ve_state.owner
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub enum ExtendVotingEscrowOrderAction {
    Extend {
        order_input_ix: u32,
        voting_escrow_input_ix: u32,
    },
    Refund,
}

impl IntoPlutusData for ExtendVotingEscrowOrderAction {
    fn into_pd(self) -> PlutusData {
        match self {
            ExtendVotingEscrowOrderAction::Extend {
                order_input_ix,
                voting_escrow_input_ix,
            } => make_constr_pd_indefinite_arr(vec![
                PlutusData::new_integer(BigInteger::from(order_input_ix)),
                PlutusData::new_integer(BigInteger::from(voting_escrow_input_ix)),
            ]),
            ExtendVotingEscrowOrderAction::Refund => {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![]))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Hash, Deserialize, PartialEq, Eq)]
pub struct ExtendVotingEscrowOrderState {
    pub ve_state: VotingEscrowConfig,
    pub ve_identifier_token_name: spectrum_cardano_lib::AssetName,
}

impl IntoPlutusData for ExtendVotingEscrowOrderState {
    fn into_pd(self) -> PlutusData {
        let ve_identifier_pd = PlutusData::new_bytes(self.ve_identifier_token_name.as_bytes().to_vec());
        let ve_state_pd = self.ve_state.into_pd();
        make_constr_pd_indefinite_arr(vec![ve_state_pd, ve_identifier_pd])
    }
}

impl TryFromPData for ExtendVotingEscrowOrderState {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let ve_state = VotingEscrowConfig::try_from_pd(cpd.take_field(0)?)?;
        let ve_ident_name_bytes = cpd.take_field(1)?.into_bytes()?;
        let ve_identifier_token_name =
            spectrum_cardano_lib::AssetName::from(AssetName::try_from(ve_ident_name_bytes).ok()?);
        Some(Self {
            ve_state,
            ve_identifier_token_name,
        })
    }
}

pub fn compute_extend_ve_order_validator(mint_composition_token_policy: PolicyId) -> PlutusV2Script {
    let ve_identifier_token_policy =
        PlutusV2Script::new(hex::decode(&DaoScriptData::global().mint_identifier.script_bytes).unwrap())
            .hash();
    let params_pd =
        uplc_pallas_primitives::PlutusData::Array(uplc_pallas_primitives::MaybeIndefArray::Indef(vec![
            uplc_pallas_primitives::PlutusData::BoundedBytes(BoundedBytes::from(
                mint_composition_token_policy.to_raw_bytes().to_vec(),
            )),
            uplc::PlutusData::BoundedBytes(BoundedBytes::from(
                ve_identifier_token_policy.to_raw_bytes().to_vec(),
            )),
        ]));
    apply_params_validator_plutus_v2(
        params_pd,
        &DaoScriptData::global().extend_voting_escrow_order.script_bytes,
    )
}
