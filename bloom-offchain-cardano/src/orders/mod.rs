use cml_multi_era::babbage::BabbageTransactionOutput;
use std::fmt::{Debug, Display, Formatter};

use bloom_derivation::{Fragment, Stable, Tradable};
use bloom_offchain::execution_engine::liquidity_book::fragment::{OrderState, StateTrans};
use bloom_offchain::execution_engine::liquidity_book::types::{ExBudgetUsed, ExFeeUsed};
use spectrum_offchain::data::{Has, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderV1;
use spectrum_offchain_cardano::utxo::ConsumedInputs;

use crate::orders::limit::{LimitOrder, LimitOrderBounds};
use crate::orders::partitioning::Partitioning;

pub mod limit;
pub mod partitioning;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Fragment, Stable, Tradable)]
pub enum AnyOrder {
    Limit(LimitOrder),
}

impl Display for AnyOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            AnyOrder::Limit(lo) => std::fmt::Display::fmt(&lo, f),
        }
    }
}

impl OrderState for AnyOrder {
    fn with_updated_time(self, time: u64) -> StateTrans<Self> {
        match self {
            AnyOrder::Limit(spot) => spot.with_updated_time(time).map(AnyOrder::Limit),
        }
    }
    fn with_applied_swap(
        self,
        removed_input: u64,
        added_output: u64,
    ) -> (StateTrans<Self>, ExBudgetUsed, ExFeeUsed) {
        match self {
            AnyOrder::Limit(spot) => {
                let (tx, budget, fee) = spot.with_applied_swap(removed_input, added_output);
                (tx.map(AnyOrder::Limit), budget, fee)
            }
        }
    }
}

impl<C> TryFromLedger<BabbageTransactionOutput, C> for AnyOrder
where
    C: Has<OperatorCred>
        + Has<ConsumedInputs>
        + Has<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>
        + Has<LimitOrderBounds>
        + Has<Partitioning>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &C) -> Option<Self> {
        match LimitOrder::try_from_ledger(repr, ctx) {
            Some(ord) if ctx.select::<Partitioning>().in_my_partition(ord.pair_id()) => {
                Some(AnyOrder::Limit(ord))
            }
            _ => None,
        }
    }
}
