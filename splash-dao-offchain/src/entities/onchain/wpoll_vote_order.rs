use cml_chain::{
    plutus::{ConstrPlutusData, PlutusData},
    transaction::TransactionOutput,
    utils::BigInteger,
    LenEncoding,
};
use cml_crypto::RawBytesEncoding;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{make_constr_pd_indefinite_arr, DatumExtension, IntoPlutusData},
    transaction::TransactionOutputExtension,
    types::TryFromPData,
    OutputRef, Token,
};
use spectrum_offchain::{
    backlog::data::{OrderWeight, Weighted},
    domain::{order::UniqueOrder, Has, Stable},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};

use crate::{
    constants::{MAKE_VOTING_ESCROW_ORDER_MIN_LOVELACES, WPOLL_VOTE_ORDER_MIN_LOVELACES},
    deployment::ProtocolValidator,
    routines::TimedOutputRef,
};

use super::{
    smart_farm::FarmId,
    voting_escrow::{Owner, VotingEscrowConfig},
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
    pub ve_datum: VotingEscrowConfig,
}

impl<C> TryFromLedger<TransactionOutput, C> for WPollVoteOnchainOrder
where
    C: Has<DeployedScriptInfo<{ ProtocolValidator::WPollVoteOrder as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            if value.coin >= WPOLL_VOTE_ORDER_MIN_LOVELACES {
                let ve_datum = VotingEscrowConfig::try_from_pd(repr.datum()?.into_pd()?)?;
                return Some(Self { ve_datum });
            }
        }
        None
    }
}

impl Stable for WPollVoteOnchainOrder {
    type StableId = Owner;

    fn stable_id(&self) -> Self::StableId {
        self.ve_datum.owner
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone)]
pub enum WPollVoteAction {
    CastVote {
        weighting_poll_auth_token: Token,
        ve_identifier_token: Token,
        voting_escrow_input_ix: u32,
        wpoll_input_ix: u32,
        expected_diff: Vec<(FarmId, u64)>,
    },
    Refund,
}

impl IntoPlutusData for WPollVoteAction {
    fn into_pd(self) -> PlutusData {
        match self {
            WPollVoteAction::CastVote {
                weighting_poll_auth_token,
                ve_identifier_token,
                voting_escrow_input_ix,
                wpoll_input_ix,
                expected_diff,
            } => {
                let wpoll_auth_token_pd = make_constr_pd_indefinite_arr(vec![
                    PlutusData::new_bytes(weighting_poll_auth_token.0.to_raw_bytes().to_vec()),
                    PlutusData::new_bytes(weighting_poll_auth_token.1.as_bytes().to_vec()),
                ]);
                let ve_identifier_pd = make_constr_pd_indefinite_arr(vec![
                    PlutusData::new_bytes(ve_identifier_token.0.to_raw_bytes().to_vec()),
                    PlutusData::new_bytes(ve_identifier_token.1.as_bytes().to_vec()),
                ]);

                let expected_diff = expected_diff
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
                    wpoll_auth_token_pd,
                    ve_identifier_pd,
                    PlutusData::new_integer(BigInteger::from(voting_escrow_input_ix)),
                    PlutusData::new_integer(BigInteger::from(wpoll_input_ix)),
                    expected_diff_pd,
                ])
            }
            WPollVoteAction::Refund => PlutusData::new_constr_plutus_data(ConstrPlutusData::new(1, vec![])),
        }
    }
}
