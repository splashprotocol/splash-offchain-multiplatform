use std::cmp::Ordering;

use spectrum_cardano_lib::TaggedAmount;

use crate::liquidity_book::types::{Base, BatcherFeePerQuote, CFMMPrice, ConstantPrice, InclusionCost, LPFee, Quote, SourceId};

#[derive(Debug, Copy, Clone)]
pub enum PoolPrice {
    /// Price is constant.
    Const(ConstantPrice),
    /// Price is a constant function.
    CFMM(CFMMPrice),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TimeBounds<T> {
    Until(T),
    After(T),
    Within(T, T),
    None,
}

impl<T> TimeBounds<T> where T: Ord + Copy {
    pub fn contains(&self, time_slot: &T) -> bool {
        match self {
            TimeBounds::Until(t) => time_slot < t,
            TimeBounds::After(t) => t >= time_slot,
            TimeBounds::Within(t0, t1) => t0 >= time_slot && time_slot < t1,
            TimeBounds::None => true,
        }
    }
    pub fn lower_bound(&self) -> Option<T> {
        match self {
            TimeBounds::After(t) => Some(*t),
            TimeBounds::Within(t0, _) => Some(*t0),
            TimeBounds::Until(_) | TimeBounds::None => None,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum OneSideLiquidity<T> {
    Bid(T),
    Ask(T),
}

impl<T> OneSideLiquidity<T> {
    pub fn any(&self) -> &T {
        match self {
            OneSideLiquidity::Bid(t) => t,
            OneSideLiquidity::Ask(t) => t,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct LiquidityFragment<T> {
    pub id: SourceId,
    pub amount: TaggedAmount<Base>,
    pub price: ConstantPrice,
    pub fee: BatcherFeePerQuote,
    pub cost: InclusionCost,
    pub bounds: TimeBounds<T>,
}

impl<T: Eq> PartialOrd for LiquidityFragment<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        todo!()
    }
}

impl<T: Eq> Ord for LiquidityFragment<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        todo!()
    }
}

#[derive(Debug, Copy, Clone)]
pub enum PooledLiquidity {
    CFMMPool(CFMMPool)
}

#[derive(Debug, Copy, Clone)]
pub struct CFMMPool {
    id: SourceId,
    reserves_base: TaggedAmount<Base>,
    reserves_quote: TaggedAmount<Quote>,
    price: CFMMPrice,
    lp_fee: LPFee,
    cost_hint: InclusionCost,
}
