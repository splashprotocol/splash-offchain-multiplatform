use num_rational::Ratio;

use crate::execution_engine::liquidity_book::side::SideM;
use crate::execution_engine::liquidity_book::time::TimeBounds;
use crate::execution_engine::liquidity_book::types::{
    AbsolutePrice, ExBudgetUsed, ExCostUnits, ExFeeUsed, FeeAsset, InputAsset, OutputAsset,
};

/// Order as a state machine.
pub trait OrderState: Sized {
    fn with_updated_time(self, time: u64) -> StateTrans<Self>;
    fn with_applied_swap(
        self,
        removed_input: InputAsset<u64>,
        added_output: OutputAsset<u64>,
    ) -> (StateTrans<Self>, ExBudgetUsed, ExFeeUsed);
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum StateTrans<T> {
    /// Next state is available.
    Active(T),
    /// Order is exhausted.
    EOL,
}

impl<T> StateTrans<T> {
    pub fn map<B, F>(self, f: F) -> StateTrans<B>
    where
        F: FnOnce(T) -> B,
    {
        match self {
            StateTrans::Active(t) => StateTrans::Active(f(t)),
            StateTrans::EOL => StateTrans::EOL,
        }
    }
}

/// Immutable discrete fragment of liquidity available at a specified timeframe at a specified price.
/// Fragment is a projection of an order [OrderState] at a specific point on time axis.
pub trait Fragment {
    /// Side of the fragment relative to pair it maps to.
    fn side(&self) -> SideM;
    fn input(&self) -> InputAsset<u64>;
    /// Price of base asset in quote asset.
    fn price(&self) -> AbsolutePrice;
    /// Batcher fee for whole swap.
    fn liner_fee(&self, input_consumed: InputAsset<u64>) -> FeeAsset<u64>;
    /// Fee value weighted by fragment size.
    fn weighted_fee(&self) -> FeeAsset<Ratio<u64>>;
    /// How much (approximately) execution of this fragment will cost.
    fn marginal_cost_hint(&self) -> ExCostUnits;
    fn time_bounds(&self) -> TimeBounds<u64>;
}
