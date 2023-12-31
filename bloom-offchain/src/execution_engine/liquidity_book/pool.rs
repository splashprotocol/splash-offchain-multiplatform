use crate::execution_engine::liquidity_book::side::Side;
use crate::execution_engine::liquidity_book::types::AbsolutePrice;
use crate::execution_engine::types::StableId;

/// Pooled liquidity.
pub trait Pool {
    /// Stable identifier of the pool.
    fn id(&self) -> StableId;
    /// Static price (regardless swap vol) in this pool.
    fn static_price(&self) -> AbsolutePrice;
    /// Real price of swap.
    fn real_price(&self, input: Side<u64>) -> AbsolutePrice;
    /// Output of a swap.
    fn swap(self, input: Side<u64>) -> (u64, Self);
    /// Quality of the pool.
    fn quality(&self) -> PoolQuality;
}

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PoolQuality(/*price hint*/ pub AbsolutePrice, /*liquidity*/ pub u64);
