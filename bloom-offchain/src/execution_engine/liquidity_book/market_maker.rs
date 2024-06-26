use std::cmp::Ordering;
use bignumber::BigNumber;

use derive_more::{Display, Div, From, Into, Mul};
use num_rational::Ratio;

use crate::execution_engine::liquidity_book::core::{Make, TryApply};
use crate::execution_engine::liquidity_book::side::Side;
use crate::execution_engine::liquidity_book::types::AbsolutePrice;

/// Price of a theoretical 0-swap in pool.
#[repr(transparent)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Div, Mul, From, Into, Display)]
pub struct SpotPrice(AbsolutePrice);

impl SpotPrice {
    pub fn unwrap(self) -> Ratio<u128> {
        self.0.unwrap()
    }
}

/// Pooled liquidity.
pub trait MarketMaker {
    type U;
    /// Static price (regardless swap vol) in this pool.
    fn static_price(&self) -> SpotPrice;
    /// Real price of swap.
    fn real_price(&self, input: Side<u64>) -> AbsolutePrice;
    /// Output of a swap.
    fn swap(self, input: Side<u64>) -> (u64, Self);
    /// Quality of the pool.
    fn quality(&self) -> PoolQuality;
    /// How much (approximately) execution of this fragment will cost.
    fn marginal_cost_hint(&self) -> Self::U;

    // Is this maker active at the moment or not.
    fn is_active(&self) -> bool;
    fn available_liquidity_by_user_impact(&self, max_user_price_impact: Side<Ratio<u64>>) -> (u64, u64);
    fn available_liquidity_by_spot_impact(&self, max_spot_price_impact: Side<Ratio<u64>>) -> (u64, u64);
}

/// Pooled liquidity.
pub trait MakerBehavior: Sized {
    /// Output of a swap.
    fn swap(self, input: Side<u64>) -> TryApply<Make, Self>;
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Into, From, Display)]
pub struct PoolQuality(u128);

impl PartialOrd for PoolQuality {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl From<u64> for PoolQuality {
    fn from(value: u64) -> Self {
        PoolQuality(value as u128)
    }
}

impl Ord for PoolQuality {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0).reverse()
    }
}
