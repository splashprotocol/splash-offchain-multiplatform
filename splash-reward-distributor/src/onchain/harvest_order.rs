use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
use spectrum_cardano_lib::{
    plutus_data::{ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension},
    transaction::TransactionOutputExtension,
    tx_view::TxViewPartiallyResolved,
    types::TryFromPData,
    OutputRef,
};
use spectrum_offchain::{domain::Has, ledger::TryFromLedger};
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::routines::{Slot, TimedOutputRef};

use crate::{config::HarvestLimits, onchain::RewardProtocolValidator};

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestOrder {
    pub lovelaces: u64,
    pub datum: HarvestOrderDatum,
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
            .map(|bytes| Ed25519KeyHash::from_raw_bytes(&bytes).unwrap())?;
        let distribution_agent_key = cpd
            .take_field(1)?
            .into_bytes()
            .map(|bytes| Ed25519KeyHash::from_raw_bytes(&bytes).unwrap())?;
        Some(Self {
            refund_key,
            distribution_agent_key,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestOrderSnapshot(pub HarvestOrder, pub TimedOutputRef);

impl<C> TryFromLedger<TxViewPartiallyResolved, C> for HarvestOrderSnapshot
where
    C: Has<HarvestLimits> + Has<DeployedScriptInfo<{ RewardProtocolValidator::HarvestOrder as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &C) -> Option<Self> {
        repr.outputs.iter().enumerate().find_map(|(ix, output)| {
            let harvest_limit = ctx.select::<HarvestLimits>().minimal_lovelace_per_single_harvest;
            let lovelaces = output.value().coin;
            if test_address(output.address(), ctx) && lovelaces >= harvest_limit {
                let datum = output.datum()?;
                let datum @ HarvestOrderDatum { .. } =
                    datum.into_pd().map(HarvestOrderDatum::try_from_pd)??;
                let harvest_order = HarvestOrder { lovelaces, datum };
                let timed_output_ref =
                    TimedOutputRef::new(OutputRef::new(repr.hash, ix as u64), Slot(repr.slot));
                return Some(HarvestOrderSnapshot(harvest_order, timed_output_ref));
            }
            None
        })
    }
}
