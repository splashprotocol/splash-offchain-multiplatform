use std::cmp::Ordering;

use crate::liquidity_book::types::{BatcherFeePerQuote, ExecutionCost, Price, SourceId};
use crate::time::TimeBounds;

/// Fragment is a part or whole liquidity of an order.
/// Fragments correspond many-to-one to orders.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Fragment<T> {
    pub source: SourceId,
    pub input: u64,
    pub price: Price,
    pub fee: BatcherFeePerQuote,
    pub cost_hint: ExecutionCost,
    pub bounds: TimeBounds<T>,
}

impl<T: PartialOrd> PartialOrd for Fragment<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        todo!()
    }
}

impl<T: Ord> Ord for Fragment<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        todo!()
    }
}
