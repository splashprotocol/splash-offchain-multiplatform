use derive_more::{From, Into};
use rand::{thread_rng, RngCore};

use crate::execution_engine::liquidity_book::side::Side;
use crate::execution_engine::liquidity_book::types::BasePrice;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Into, From)]
pub struct PoolId([u8; 32]);
impl PoolId {
    #[cfg(test)]
    pub fn random() -> PoolId {
        let mut bf = [0u8; 32];
        thread_rng().fill_bytes(&mut bf);
        Self(bf)
    }
}

/// Pooled liquidity.
pub trait Pool {
    /// Stable identifier of the pool.
    fn id(&self) -> PoolId;
    /// Static price (regardless swap vol) in this pool.
    fn static_price(&self) -> BasePrice;
    /// Real price of swap.
    fn real_price(&self, input: Side<u64>) -> BasePrice;
    /// Output of a swap.
    fn swap(self, input: Side<u64>) -> (u64, Self);
    /// Quality of the pool.
    fn quality(&self) -> PoolQuality;
}

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PoolQuality(/*price hint*/ pub BasePrice, /*liquidity*/ pub u64);
