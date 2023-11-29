use crate::side::Side;
use crate::types::Price;

/// Pooled liquidity.
pub trait Pool {
    /// Static price (regardless swap vol) in this pool.
    fn static_price(&self) -> Price;
    /// Real price of swap.
    fn real_price(&self, input: Side<u64>) -> Price;
    /// Output of a swap.
    fn output(&self, input: Side<u64>) -> u64;
}
