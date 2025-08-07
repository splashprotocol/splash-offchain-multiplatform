use cml_chain::{
    certs::{Credential, StakeCredential},
    plutus::ConstrPlutusData,
    transaction::TransactionOutput,
};
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension},
    transaction::TransactionOutputExtension,
    tx_view::{TimedOutput, TxViewPartiallyResolved},
    types::TryFromPData,
    OutputRef,
};
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::{deployment::ProtocolValidator as DaoProtocolValidator, routines::Slot};

use crate::config::HarvestLimits;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarvestOrder<OrderId> {
    pub id: OrderId,
    pub account: Ed25519KeyHash,
    pub issued_at: Slot,
    pub owner_stake_credential: Option<StakeCredential>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestOrderCredential(Credential);

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestOrderDatum {
    refund_key: Ed25519KeyHash,
    owner_stake_credential: Option<StakeCredential>,
    distribution_agent_key: Ed25519KeyHash,
}

impl TryFromPData for HarvestOrderDatum {
    fn try_from_pd(data: cml_chain::plutus::PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let refund_key = cpd
            .take_field(0)?
            .into_bytes()
            .map(|bytes| Ed25519KeyHash::from_raw_bytes(&bytes).ok())??;

        let mut option_cpd = cpd.take_field(1)?.into_constr_pd()?;
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
        let distribution_agent_key = cpd
            .take_field(2)?
            .into_bytes()
            .map(|bytes| Ed25519KeyHash::from_raw_bytes(&bytes).ok())??;
        Some(Self {
            refund_key,
            owner_stake_credential,
            distribution_agent_key,
        })
    }
}

pub enum HarvestOrderAction {
    Refund,
    Harvest,
}

impl IntoPlutusData for HarvestOrderAction {
    fn into_pd(self) -> cml_chain::plutus::PlutusData {
        let alternative = match self {
            HarvestOrderAction::Refund => 0,
            HarvestOrderAction::Harvest => 1,
        };
        cml_chain::plutus::PlutusData::ConstrPlutusData(ConstrPlutusData::new(alternative, vec![]))
    }
}

/// Try to extract a newly-created harvest order. If multiple orders exist in this TX we take the
/// first one and ignore subsequent orders.
pub(crate) fn try_new_harvest_request<C>(
    repr: &TxViewPartiallyResolved,
    ctx: &C,
) -> Option<HarvestOrder<OutputRef>>
where
    C: Has<HarvestLimits> + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>,
{
    repr.outputs.iter().enumerate().find_map(|(ix, output)| {
        let output_ref = OutputRef::new(repr.hash, ix as u64);
        try_extract_harvest_order(output, output_ref, Slot(repr.slot), ctx)
    })
}

/// Returns the OutputRefs of all known harvest orders that have been consumed.
pub(crate) fn get_consumed_harvest_orders<C>(
    repr: &TxViewPartiallyResolved,
    ctx: &C,
) -> Vec<HarvestOrder<OutputRef>>
where
    C: Has<HarvestLimits> + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>,
{
    repr.inputs
        .iter()
        .filter_map(|(tx_input, output)| {
            if let Some(TimedOutput { output, slot }) = output {
                let output_ref = OutputRef::from(tx_input.clone());
                return try_extract_harvest_order(output, output_ref, Slot(*slot), ctx);
            }
            None
        })
        .collect()
}

fn try_extract_harvest_order<C>(
    output: &TransactionOutput,
    output_ref: OutputRef,
    issued_at: Slot,
    ctx: &C,
) -> Option<HarvestOrder<OutputRef>>
where
    C: Has<HarvestLimits> + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>,
{
    let harvest_limit = ctx.select::<HarvestLimits>().minimal_lovelace_per_single_harvest;
    let lovelaces = output.value().coin;
    if test_address(output.address(), ctx) && lovelaces >= harvest_limit {
        let datum = output.datum()?;
        let HarvestOrderDatum {
            refund_key,
            owner_stake_credential,
            ..
        } = datum.into_pd().map(HarvestOrderDatum::try_from_pd)??;
        let harvest_order = HarvestOrder {
            id: output_ref,
            account: refund_key,
            issued_at,
            owner_stake_credential,
        };
        return Some(harvest_order);
    }
    None
}
