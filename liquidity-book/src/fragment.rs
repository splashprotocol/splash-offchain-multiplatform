use crate::time::TimeBounds;
use crate::types::{ExecutionCost, Price, SourceId};

/// Discrete fragment of liquidity available at a specified timeframe at a specified price.
pub trait Fragment {
    fn source(&self) -> SourceId;
    fn input(&self) -> u64;
    fn price(&self) -> Price;
    fn weight(&self) -> u64;
    fn cost_hint(&self) -> ExecutionCost;
    fn time_bounds(&self) -> TimeBounds<u64>;
}
