use cml_chain::{
    auxdata::Metadata,
    plutus::{ConstrPlutusData, PlutusData, PlutusV2Script},
    transaction::TransactionOutput,
    utils::BigInteger,
    LenEncoding, PolicyId,
};
use cml_crypto::RawBytesEncoding;
use log::trace;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{
        make_constr_pd_indefinite_arr, ConstrPlutusDataExtension, DatumExtension, IntoPlutusData,
        PlutusDataExtension,
    },
    transaction::TransactionOutputExtension,
    types::TryFromPData,
    AssetName, OutputRef, Token,
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

use crate::{
    constants::WPOLL_VOTE_ORDER_MIN_LOVELACES,
    deployment::{DaoScriptData, ProtocolValidator},
    routines::TimedOutputRef,
};

use super::{
    get_proxy_order_metadata,
    smart_farm::FarmId,
    voting_escrow::{Owner, VotingEscrowConfig},
    ProxyOrderMetadata,
};

#[derive(Hash, PartialEq, Eq, Serialize, Deserialize, Clone, Debug)]
pub struct WPollVoteOrderBundle<Bearer> {
    pub order: WPollVoteOnchainOrder,
    pub output_ref: TimedOutputRef,
    pub bearer: Bearer,
}

impl<Bearer> WPollVoteOrderBundle<Bearer> {
    pub fn new(order: WPollVoteOnchainOrder, output_ref: TimedOutputRef, bearer: Bearer) -> Self {
        Self {
            order,
            output_ref,
            bearer,
        }
    }
}

impl<Bearer> UniqueOrder for WPollVoteOrderBundle<Bearer> {
    type TOrderId = OutputRef;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.output_ref.output_ref
    }
}

impl<Bearer> Weighted for WPollVoteOrderBundle<Bearer> {
    fn weight(&self) -> OrderWeight {
        // Older orders first
        OrderWeight::from(u64::MAX - self.output_ref.slot.0)
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub struct WPollVoteOnchainOrder {
    pub datum: WPollVoteState,
    pub metadata: ProxyOrderMetadata,
}

impl<C> TryFromLedger<TransactionOutput, C> for WPollVoteOnchainOrder
where
    C: Has<DeployedScriptInfo<{ ProtocolValidator::WPollVoteOrder as u8 }>> + Has<Option<Metadata>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            if value.coin >= WPOLL_VOTE_ORDER_MIN_LOVELACES {
                let datum = WPollVoteState::try_from_pd(repr.datum()?.into_pd()?)?;
                let tx_metadata = ctx.select::<Option<Metadata>>()?;
                let metadata = get_proxy_order_metadata(tx_metadata)?;
                return Some(Self { datum, metadata });
            }
        }
        None
    }
}

impl Stable for WPollVoteOnchainOrder {
    type StableId = Owner;

    fn stable_id(&self) -> Self::StableId {
        self.datum.ve_state.owner
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone)]
pub enum WPollVoteAction {
    CastVote {
        voting_escrow_input_ix: u32,
        wpoll_input_ix: u32,
    },
    Refund,
}

impl IntoPlutusData for WPollVoteAction {
    fn into_pd(self) -> PlutusData {
        match self {
            WPollVoteAction::CastVote {
                voting_escrow_input_ix,
                wpoll_input_ix,
            } => make_constr_pd_indefinite_arr(vec![
                PlutusData::new_integer(BigInteger::from(voting_escrow_input_ix)),
                PlutusData::new_integer(BigInteger::from(wpoll_input_ix)),
            ]),
            WPollVoteAction::Refund => PlutusData::new_constr_plutus_data(ConstrPlutusData::new(1, vec![])),
        }
    }
}

#[derive(Debug, Clone, Serialize, Hash, Deserialize, PartialEq, Eq)]
pub struct WPollVoteState {
    pub ve_state: VotingEscrowConfig,
    pub weighting_poll_auth_token_name: AssetName,
    pub ve_identifier_token_name: AssetName,
    pub expected_diff: Vec<(FarmId, u64)>,
}

impl IntoPlutusData for WPollVoteState {
    fn into_pd(self) -> PlutusData {
        let wpoll_auth_token_pd =
            PlutusData::new_bytes(self.weighting_poll_auth_token_name.as_bytes().to_vec());
        let ve_identifier_pd = PlutusData::new_bytes(self.ve_identifier_token_name.as_bytes().to_vec());
        let ve_state_pd = self.ve_state.into_pd();
        let expected_diff = self
            .expected_diff
            .iter()
            .map(|&(farm_id, weight)| {
                make_constr_pd_indefinite_arr(vec![
                    PlutusData::new_bytes(cml_chain::assets::AssetName::from(farm_id.0).inner),
                    PlutusData::new_integer(BigInteger::from(weight)),
                ])
            })
            .collect();

        let expected_diff_pd = PlutusData::List {
            list: expected_diff,
            list_encoding: LenEncoding::Indefinite,
        };
        make_constr_pd_indefinite_arr(vec![
            ve_state_pd,
            wpoll_auth_token_pd,
            ve_identifier_pd,
            expected_diff_pd,
        ])
    }
}

impl TryFromPData for WPollVoteState {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let ve_state = VotingEscrowConfig::try_from_pd(cpd.take_field(0)?)?;
        let wpoll_auth_token_name_bytes = cpd.take_field(1)?.into_bytes()?;
        let weighting_poll_auth_token_name = AssetName::try_from(wpoll_auth_token_name_bytes).ok()?;
        let ve_ident_name_bytes = cpd.take_field(2)?.into_bytes()?;
        let expected_diff = cpd.take_field(3)?.into_vec_pd(|pd| {
            let mut pair = pd.into_constr_pd()?;
            let asset_name_bytes = pair.take_field(0)?.into_bytes()?;
            let asset_name = FarmId(AssetName::try_from(asset_name_bytes).ok()?);
            let weight = pair.take_field(1)?.into_u64()?;
            Some((asset_name, weight))
        })?;
        let ve_identifier_token_name = AssetName::try_from(ve_ident_name_bytes).ok()?;
        Some(Self {
            ve_state,
            weighting_poll_auth_token_name,
            ve_identifier_token_name,
            expected_diff,
        })
    }
}

pub fn compute_wpoll_vote_order_validator(wpoll_auth_token_policy: PolicyId) -> PlutusV2Script {
    let ve_identifier_token_policy =
        PlutusV2Script::new(hex::decode(&DaoScriptData::global().mint_identifier.script_bytes).unwrap())
            .hash();
    let params_pd = uplc::PlutusData::Array(MaybeIndefArray::Indef(vec![
        uplc::PlutusData::BoundedBytes(BoundedBytes::from(
            wpoll_auth_token_policy.to_raw_bytes().to_vec(),
        )),
        uplc::PlutusData::BoundedBytes(BoundedBytes::from(
            ve_identifier_token_policy.to_raw_bytes().to_vec(),
        )),
    ]));
    apply_params_validator_plutus_v2(params_pd, &DaoScriptData::global().wpoll_vote_order.script_bytes)
}
