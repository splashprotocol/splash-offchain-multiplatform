use crate::execution_engine::liquidity_book::side::Side;
use crate::execution_engine::liquidity_book::time::TimeBounds;
use crate::execution_engine::liquidity_book::types::{ExecutionCost, Price};
use crate::execution_engine::SourceId;

/// Discrete fragment of liquidity available at a specified timeframe at a specified price.
/// Fragment is a projection of an order at a specific point on time axis.
pub trait Fragment: Sized {
    fn source(&self) -> SourceId;
    fn input(&self) -> u64;
    fn price(&self) -> Price;
    fn weight(&self) -> u64;
    fn cost_hint(&self) -> ExecutionCost;
    fn time_bounds(&self) -> TimeBounds<u64>;
    fn advance_time(self, time: u64) -> Option<Self>;
    fn satisfy(self, output: u64) -> Option<Side<Self>>;
}
