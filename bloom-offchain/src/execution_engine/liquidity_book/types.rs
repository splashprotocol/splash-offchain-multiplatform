use num_rational::Ratio;

use crate::execution_engine::liquidity_book::side::Side;

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
