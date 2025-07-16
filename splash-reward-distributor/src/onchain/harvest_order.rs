use cml_chain::{certs::Credential, transaction::TransactionOutput};
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
use spectrum_cardano_lib::{
    plutus_data::{ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension},
    transaction::TransactionOutputExtension,
    tx_view::TxViewPartiallyResolved,
    types::TryFromPData,
    OutputRef,
};
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::deployment::ProtocolValidator as DaoProtocolValidator;

use crate::config::HarvestLimits;

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestOrder<OrderId> {
    pub id: OrderId,
    pub account: Credential,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestOrderDatum {
    refund_key: Ed25519KeyHash,
    distribution_agent_key: Ed25519KeyHash,
}

impl TryFromPData for HarvestOrderDatum {
    fn try_from_pd(data: cml_chain::plutus::PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let refund_key = cpd
            .take_field(0)?
            .into_bytes()
            .map(|bytes| Ed25519KeyHash::from_raw_bytes(&bytes).ok())??;
        let distribution_agent_key = cpd
            .take_field(1)?
            .into_bytes()
            .map(|bytes| Ed25519KeyHash::from_raw_bytes(&bytes).ok())??;
        Some(Self {
            refund_key,
            distribution_agent_key,
        })
    }
}

/// Try to extract a newly-created harvest order.
pub(crate) fn try_new_harvest_request<C>(
    repr: &TxViewPartiallyResolved,
    ctx: &C,
) -> Option<HarvestOrder<OutputRef>>
where
    C: Has<HarvestLimits> + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>,
{
    let harvest_order_out = repr.outputs.iter().enumerate().find_map(|(ix, output)| {
        let output_ref = OutputRef::new(repr.hash, ix as u64);
        extract_harvest_order(output, output_ref, ctx)
    })?;
    let no_harvest_order_input = repr.inputs.iter().all(|(tx_input, output)| {
        if let Some(output) = output {
            let output_ref = OutputRef::from(tx_input.clone());
            return extract_harvest_order(output, output_ref, ctx).is_none();
        }
        true
    });
    if no_harvest_order_input {
        Some(harvest_order_out)
    } else {
        None
    }
}

/// Returns the OutputRefs of all known harvest orders that have been consumed.
pub(crate) fn get_consumed_harvest_orders<C>(repr: &TxViewPartiallyResolved, ctx: &C) -> Vec<OutputRef>
where
    C: Has<HarvestLimits> + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>,
{
    repr.inputs
        .iter()
        .filter_map(|(tx_input, output)| {
            if let Some(output) = output {
                let output_ref = OutputRef::from(tx_input.clone());
                return extract_harvest_order(output, output_ref, ctx).map(|order| order.id);
            }
            None
        })
        .collect()
}

fn extract_harvest_order<C>(
    output: &TransactionOutput,
    output_ref: OutputRef,
    ctx: &C,
) -> Option<HarvestOrder<OutputRef>>
where
    C: Has<HarvestLimits> + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>,
{
    let harvest_limit = ctx.select::<HarvestLimits>().minimal_lovelace_per_single_harvest;
    let lovelaces = output.value().coin;
    if test_address(output.address(), ctx) && lovelaces >= harvest_limit {
        let datum = output.datum()?;
        let HarvestOrderDatum { refund_key, .. } = datum.into_pd().map(HarvestOrderDatum::try_from_pd)??;
        let account = Credential::new_pub_key(refund_key);
        let harvest_order = HarvestOrder {
            id: output_ref,
            account,
        };
        return Some(harvest_order);
    }
    None
}
