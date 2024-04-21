use crate::execution_engine::liquidity_book::side::Side;
use crate::execution_engine::liquidity_book::types::AbsolutePrice;
use derive_more::{From, Into};
use std::cmp::Ordering;

/// Pooled liquidity.
pub trait Pool {
    type U;
    /// Static price (regardless swap vol) in this pool.
    fn static_price(&self) -> AbsolutePrice;
    /// Real price of swap.
    fn real_price(&self, input: Side<u64>) -> AbsolutePrice;
    /// Output of a swap.
    fn swap(self, input: Side<u64>) -> (u64, Self);
    /// Quality of the pool.
    fn quality(&self) -> PoolQuality;
    /// How much (approximately) execution of this fragment will cost.
    fn marginal_cost_hint(&self) -> Self::U;
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Into, From)]
pub struct PoolQuality(u64);

impl PartialOrd for PoolQuality {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PoolQuality {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0).reverse()
    }
}
