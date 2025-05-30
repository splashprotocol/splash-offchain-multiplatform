use cml_chain::transaction::TransactionOutput;
use std::fmt::{Debug, Display, Formatter};

use crate::orders::grid::GridOrder;
use crate::orders::limit::{LimitOrder, LimitOrderValidation};
use bloom_derivation::{MarketTaker, Stable, Tradable};
use bloom_offchain::execution_engine::liquidity_book::core::{Next, TerminalTake, Unit};
use bloom_offchain::execution_engine::liquidity_book::market_taker::TakerBehaviour;
use bloom_offchain::execution_engine::liquidity_book::types::{InputAsset, OutputAsset, RelativePrice};
use spectrum_cardano_lib::{OutputRef, Token};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderV1;
use spectrum_offchain_cardano::handler_context::{ConsumedIdentifiers, ConsumedInputs, ProducedIdentifiers};

pub mod adhoc;
pub mod grid;
pub mod instant;
pub mod limit;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, MarketTaker, Stable, Tradable)]
pub enum AnyOrder {
    Limit(LimitOrder),
    Grid(GridOrder),
}

impl Display for AnyOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            AnyOrder::Limit(lo) => std::fmt::Display::fmt(&lo, f),
            AnyOrder::Grid(go) => std::fmt::Display::fmt(&go, f),
        }
    }
}

impl TakerBehaviour for AnyOrder {
    fn with_updated_time(self, time: u64) -> Next<Self, Unit> {
        match self {
            AnyOrder::Limit(o) => o.with_updated_time(time).map_succ(AnyOrder::Limit),
            AnyOrder::Grid(o) => o.with_updated_time(time).map_succ(AnyOrder::Grid),
        }
    }

    fn with_applied_trade(
        self,
        removed_input: InputAsset<u64>,
        added_output: OutputAsset<u64>,
    ) -> Next<Self, TerminalTake> {
        match self {
            AnyOrder::Limit(o) => o
                .with_applied_trade(removed_input, added_output)
                .map_succ(AnyOrder::Limit),
            AnyOrder::Grid(o) => o
                .with_applied_trade(removed_input, added_output)
                .map_succ(AnyOrder::Grid),
        }
    }

    fn with_budget_corrected(self, delta: i64) -> (i64, Self) {
        match self {
            AnyOrder::Limit(o) => {
                let (d, s) = o.with_budget_corrected(delta);
                (d, AnyOrder::Limit(s))
            }
            AnyOrder::Grid(o) => {
                let (d, s) = o.with_budget_corrected(delta);
                (d, AnyOrder::Grid(s))
            }
        }
    }

    fn with_fee_charged(self, fee: u64) -> Self {
        match self {
            AnyOrder::Limit(o) => AnyOrder::Limit(o.with_fee_charged(fee)),
            AnyOrder::Grid(o) => AnyOrder::Grid(o.with_fee_charged(fee)),
        }
    }

    fn with_output_added(self, added_output: u64) -> Self {
        match self {
            AnyOrder::Limit(o) => AnyOrder::Limit(o.with_output_added(added_output)),
            AnyOrder::Grid(o) => AnyOrder::Grid(o.with_output_added(added_output)),
        }
    }

    fn try_terminate(self) -> Next<Self, TerminalTake> {
        match self {
            AnyOrder::Limit(o) => o.try_terminate().map_succ(AnyOrder::Limit),
            AnyOrder::Grid(o) => o.try_terminate().map_succ(AnyOrder::Grid),
        }
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for AnyOrder
where
    C: Has<OperatorCred>
        + Has<OutputRef>
        + Has<ConsumedIdentifiers<Token>>
        + Has<ProducedIdentifiers<Token>>
        + Has<ConsumedInputs>
        + Has<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>
        + Has<LimitOrderValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        LimitOrder::try_from_ledger(repr, ctx).map(AnyOrder::Limit)
    }
}

const MAX_PRICE_DENOM: u128 = 1125899906842624;

pub(crate) fn harden_price(p: RelativePrice, input: u64) -> RelativePrice {
    let min_output = (input as u128 * *p.numer()).div_ceil(*p.denom());
    if min_output > 0 {
        RelativePrice::new(min_output, input as u128)
    } else {
        RelativePrice::new(1, MAX_PRICE_DENOM)
    }
}
