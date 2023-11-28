use crate::side::SideMarker;
use crate::types::Price;

/// Arbitrary liquidity pool.
pub trait Pool {
    /// Static price (regardless swap vol) in this pool.
    fn static_price(&self) -> Price;
    /// Real price of swap.
    fn real_price(&self, input: u64) -> Price;
    /// Output of a swap.
    fn output(&self, side: SideMarker, input: u64) -> u64;
}
