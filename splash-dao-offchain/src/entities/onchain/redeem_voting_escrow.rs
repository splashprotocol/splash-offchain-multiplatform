use cml_chain::{
    auxdata::Metadata,
    certs::StakeCredential,
    plutus::{ConstrPlutusData, PlutusData, PlutusV3Script},
    transaction::TransactionOutput,
    utils::BigInteger,
    PolicyId,
};
use cml_crypto::RawBytesEncoding;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{make_constr_pd_indefinite_arr, DatumExtension, IntoPlutusData},
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
    parametrized_validators::apply_params_validator_plutus_v3,
};
use uplc_pallas_primitives::{BoundedBytes, MaybeIndefArray};

use crate::{
    constants::REDEEM_VOTING_ESCROW_ORDER_MIN_LOVELACES,
    deployment::{DaoScriptData, ProtocolValidator},
    routines::TimedOutputRef,
};

use super::{
    get_proxy_order_metadata,
    voting_escrow::{Owner, VotingEscrowConfig},
    ProxyOrderMetadata,
};

#[derive(Hash, PartialEq, Eq, Serialize, Deserialize, Clone, Debug)]
pub struct RedeemVotingEscrowOrderBundle<Bearer> {
    pub order: RedeemVotingEscrowOnchainOrder,
    pub output_ref: TimedOutputRef,
    pub bearer: Bearer,
}

impl<Bearer> RedeemVotingEscrowOrderBundle<Bearer> {
    pub fn new(order: RedeemVotingEscrowOnchainOrder, output_ref: TimedOutputRef, bearer: Bearer) -> Self {
        Self {
            order,
            output_ref,
            bearer,
        }
    }
}

impl<Bearer> UniqueOrder for RedeemVotingEscrowOrderBundle<Bearer> {
    type TOrderId = OutputRef;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.output_ref.output_ref
    }
}

impl<Bearer> Weighted for RedeemVotingEscrowOrderBundle<Bearer> {
    fn weight(&self) -> OrderWeight {
        // Older orders first
        OrderWeight::from(u64::MAX - self.output_ref.slot.0)
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub struct RedeemVotingEscrowOnchainOrder {
    pub ve_datum: VotingEscrowConfig,
    pub metadata: ProxyOrderMetadata,
}

impl<C> TryFromLedger<TransactionOutput, C> for RedeemVotingEscrowOnchainOrder
where
    C: Has<DeployedScriptInfo<{ ProtocolValidator::RedeemVeOrder as u8 }>> + Has<Option<Metadata>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            if value.coin >= REDEEM_VOTING_ESCROW_ORDER_MIN_LOVELACES {
                let ve_datum = VotingEscrowConfig::try_from_pd(repr.datum()?.into_pd()?)?;
                let tx_metadata = ctx.select::<Option<Metadata>>()?;
                let metadata = get_proxy_order_metadata(tx_metadata)?;
                return Some(Self { ve_datum, metadata });
            }
        }
        None
    }
}

impl Stable for RedeemVotingEscrowOnchainOrder {
    type StableId = Owner;

    fn stable_id(&self) -> Self::StableId {
        self.ve_datum.owner
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub enum RedeemVEOrderAction {
    RedeemVE {
        ve_identifier_token_name: AssetName,
        owner_stake_credential: Option<StakeCredential>,
        voting_escrow_input_ix: u32,
        ve_factory_input_ix: u32,
    },
    Refund,
}

impl IntoPlutusData for RedeemVEOrderAction {
    fn into_pd(self) -> PlutusData {
        match self {
            RedeemVEOrderAction::RedeemVE {
                ve_identifier_token_name,
                owner_stake_credential,
                voting_escrow_input_ix,
                ve_factory_input_ix,
            } => {
                let ve_identifier_pd = PlutusData::new_bytes(ve_identifier_token_name.as_bytes().to_vec());
                let stake_cred_pd = if let Some(sc) = owner_stake_credential {
                    make_constr_pd_indefinite_arr(vec![make_constr_pd_indefinite_arr(vec![
                        make_constr_pd_indefinite_arr(vec![PlutusData::new_bytes(
                            sc.to_raw_bytes().to_vec(),
                        )]),
                    ])])
                } else {
                    PlutusData::new_constr_plutus_data(ConstrPlutusData::new(1, vec![]))
                };
                let ve_ix = PlutusData::new_integer(BigInteger::from(voting_escrow_input_ix));
                let ve_fac_ix = PlutusData::new_integer(BigInteger::from(ve_factory_input_ix));
                make_constr_pd_indefinite_arr(vec![ve_identifier_pd, stake_cred_pd, ve_ix, ve_fac_ix])
            }
            RedeemVEOrderAction::Refund => {
                PlutusData::new_constr_plutus_data(ConstrPlutusData::new(1, vec![]))
            }
        }
    }
}

pub fn compute_redeem_ve_order_validator(
    mint_ve_composition_token_policy: PolicyId,
    splash_token_policy: PolicyId,
    ve_identifier_token_policy: PolicyId,
    ve_factory_auth_token_policy: PolicyId,
) -> PlutusV3Script {
    let pd_bytes =
        |p: PolicyId| uplc::PlutusData::BoundedBytes(BoundedBytes::from(p.to_raw_bytes().to_vec()));
    let params_pd = uplc::PlutusData::Array(MaybeIndefArray::Indef(vec![
        pd_bytes(mint_ve_composition_token_policy),
        pd_bytes(splash_token_policy),
        pd_bytes(ve_identifier_token_policy),
        pd_bytes(ve_factory_auth_token_policy),
    ]));
    apply_params_validator_plutus_v3(
        params_pd,
        &DaoScriptData::global().redeem_voting_escrow_order.script_bytes,
    )
}
