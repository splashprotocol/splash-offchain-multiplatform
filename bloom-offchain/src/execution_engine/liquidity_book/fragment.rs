use crate::execution_engine::liquidity_book::time::TimeBounds;
use crate::execution_engine::liquidity_book::types::{ExecutionCost, Price};
use crate::execution_engine::SourceId;

/// Discrete fragment of liquidity available at a specified timeframe at a specified price.
pub trait Fragment {
    fn source(&self) -> SourceId;
    fn input(&self) -> u64;
    fn price(&self) -> Price;
    fn weight(&self) -> u64;
    fn cost_hint(&self) -> ExecutionCost;
    fn time_bounds(&self) -> TimeBounds<u64>;
}
