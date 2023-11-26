use num_rational::Ratio;

use crate::liquidity_book::side::Side;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct SourceId([u8; 32]);

pub type ExecutionCost = u32;

pub type Price = Ratio<u128>;

impl Side<Price> {
    pub fn overlaps(self, that: Price) -> bool {
        match self {
            Side::Bid(this) => this >= that,
            Side::Ask(this) => this <= that,
        }
    }
    pub fn better_than(self, that: Price) -> bool {
        match self {
            Side::Bid(this) => this <= that,
            Side::Ask(this) => this >= that,
        }
    }
}

pub type BatcherFeePerQuote = Ratio<u64>;

pub type LPFee = Ratio<u64>;
