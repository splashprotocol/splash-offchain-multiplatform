use cml_chain::{
    auxdata::Metadata,
    certs::StakeCredential,
    plutus::{ConstrPlutusData, PlutusData, PlutusV3Script},
    transaction::TransactionOutput,
    utils::BigInteger,
    PolicyId,
};
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
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
    pub datum: RedeemVotingEscrowOrderState,
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
                let datum = RedeemVotingEscrowOrderState::try_from_pd(repr.datum()?.into_pd()?)?;
                let tx_metadata = ctx.select::<Option<Metadata>>()?;
                let metadata = get_proxy_order_metadata(tx_metadata)?;
                return Some(Self { datum, metadata });
            }
        }
        None
    }
}

impl Stable for RedeemVotingEscrowOnchainOrder {
    type StableId = Owner;

    fn stable_id(&self) -> Self::StableId {
        self.datum.ve_state.owner
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub struct RedeemVotingEscrowOrderState {
    pub ve_state: VotingEscrowConfig,
    pub ve_identifier_token_name: AssetName,
    pub owner_stake_credential: Option<StakeCredential>,
}

impl IntoPlutusData for RedeemVotingEscrowOrderState {
    fn into_pd(self) -> PlutusData {
        let ve_state_pd = self.ve_state.into_pd();
        let ve_identifier_pd = PlutusData::new_bytes(self.ve_identifier_token_name.as_bytes().to_vec());
        let stake_cred_pd = if let Some(sc) = self.owner_stake_credential {
            make_constr_pd_indefinite_arr(vec![make_constr_pd_indefinite_arr(vec![
                make_constr_pd_indefinite_arr(vec![PlutusData::new_bytes(sc.to_raw_bytes().to_vec())]),
            ])])
        } else {
            PlutusData::new_constr_plutus_data(ConstrPlutusData::new(1, vec![]))
        };
        make_constr_pd_indefinite_arr(vec![ve_state_pd, ve_identifier_pd, stake_cred_pd])
    }
}

impl TryFromPData for RedeemVotingEscrowOrderState {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let ve_state = VotingEscrowConfig::try_from_pd(cpd.take_field(0)?)?;
        let ve_ident_name_bytes = cpd.take_field(1)?.into_bytes()?;
        let ve_identifier_token_name = AssetName::try_from(ve_ident_name_bytes).ok()?;

        // Extract stake cred. Note its Aiken representation is Option<Referenced<Credential>>

        // For Option<..>
        let mut option_cpd = cpd.take_field(2)?.into_constr_pd()?;

        let owner_stake_credential = if option_cpd.alternative == 1 {
            None
        } else {
            let mut referenced_cpd = option_cpd.take_field(0)?.into_constr_pd()?;
            // Looking for Referenced::Inline(..)
            if referenced_cpd.alternative == 0 {
                let mut stake_cred_cpd = referenced_cpd.take_field(0)?.into_constr_pd()?;
                // Expecting key hash
                if stake_cred_cpd.alternative == 0 {
                    let key_hash =
                        Ed25519KeyHash::from_raw_bytes(&stake_cred_cpd.take_field(0)?.into_bytes()?).ok()?;
                    Some(StakeCredential::new_pub_key(key_hash))
                } else {
                    // Script not supported
                    return None;
                }
            } else {
                // Referenced::Pointer { .. } not supported
                return None;
            }
        };

        Some(Self {
            ve_state,
            ve_identifier_token_name,
            owner_stake_credential,
        })
    }
}

#[derive(Clone)]
pub enum RedeemVEOrderAction {
    RedeemVE {
        voting_escrow_input_ix: u32,
        ve_factory_input_ix: u32,
    },
    Refund,
}

impl IntoPlutusData for RedeemVEOrderAction {
    fn into_pd(self) -> PlutusData {
        match self {
            RedeemVEOrderAction::RedeemVE {
                voting_escrow_input_ix,
                ve_factory_input_ix,
            } => {
                let ve_ix = PlutusData::new_integer(BigInteger::from(voting_escrow_input_ix));
                let ve_fac_ix = PlutusData::new_integer(BigInteger::from(ve_factory_input_ix));
                make_constr_pd_indefinite_arr(vec![ve_ix, ve_fac_ix])
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
