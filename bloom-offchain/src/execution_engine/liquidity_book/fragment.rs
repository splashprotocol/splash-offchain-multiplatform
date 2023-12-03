use crate::execution_engine::liquidity_book::side::SideMarker;
use crate::execution_engine::liquidity_book::time::TimeBounds;
use crate::execution_engine::liquidity_book::types::{ExecutionCost, Price};
use crate::execution_engine::SourceId;

/// Mutable state of an order.
pub trait OrderState: Sized {
    fn with_updated_time(self, time: u64) -> StateTrans<Self>;
    fn with_updated_liquidity(self, removed_input: u64, added_output: u64) -> StateTrans<Self>;
}

#[derive(Debug, Copy, Clone)]
pub enum StateTrans<T> {
    Active(T),
    EOL,
}

/// Immutable discrete fragment of liquidity available at a specified timeframe at a specified price.
/// Fragment is a projection of an order [OrderState] at a specific point on time axis.
pub trait Fragment {
    fn source(&self) -> SourceId; // todo remove
    fn side(&self) -> SideMarker;
    fn input(&self) -> u64;
    fn price(&self) -> Price;
    fn weight(&self) -> u64;
    fn cost_hint(&self) -> ExecutionCost;
    fn time_bounds(&self) -> TimeBounds<u64>;
}
