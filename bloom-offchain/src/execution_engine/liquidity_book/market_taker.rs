use crate::execution_engine::liquidity_book::core::{Next, TerminalTake, Unit};
use crate::execution_engine::liquidity_book::side::Side;
use crate::execution_engine::liquidity_book::time::TimeBounds;
use crate::execution_engine::liquidity_book::types::{AbsolutePrice, FeeAsset, InputAsset, OutputAsset};

/// Order as a state machine.
pub trait TakerBehaviour: Sized {
    fn with_updated_time(self, time: u64) -> Next<Self, Unit>;
    fn with_applied_trade(
        self,
        removed_input: InputAsset<u64>,
        added_output: OutputAsset<u64>,
    ) -> Next<Self, TerminalTake>;
    fn with_budget_corrected(self, delta: i64) -> (i64, Self);
    fn with_fee_charged(self, fee: u64) -> Self;
    fn with_output_added(self, added_output: u64) -> Self;
    fn try_terminate(self) -> Next<Self, TerminalTake>;
}

/// Immutable discrete fragment of liquidity available at a specified timeframe at a specified price.
/// MarketTaker is a projection of an order [TakerBehaviour] at a specific point on time axis.
pub trait MarketTaker {
    /// Quantifier of execution cost.
    type U;
    /// Side of the fragment relative to pair it maps to.
    fn side(&self) -> Side;
    /// Amount of input asset remaining.
    fn input(&self) -> InputAsset<u64>;
    /// Amount of output asset accumulated.
    fn output(&self) -> OutputAsset<u64>;
    /// Price of base asset in quote asset.
    fn price(&self) -> AbsolutePrice;
    /// Batcher fee for whole swap.
    fn operator_fee(&self, input_consumed: InputAsset<u64>) -> FeeAsset<u64>;
    /// Amount of fee asset reserved as operator premium.
    fn fee(&self) -> FeeAsset<u64>;
    /// Amount of fee asset reserved to pay for execution.
    fn budget(&self) -> FeeAsset<u64>;
    fn consumable_budget(&self) -> FeeAsset<u64>;
    /// How much (approximately) execution of this fragment will cost.
    fn marginal_cost_hint(&self) -> Self::U;
    /// Minimal amount of output per execution step.
    fn min_marginal_output(&self) -> OutputAsset<u64>;
    /// Time bounds of the fragment.
    fn time_bounds(&self) -> TimeBounds<u64>;
}
