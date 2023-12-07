use num_rational::Ratio;

use crate::execution_engine::liquidity_book::side::Side;

pub type ExecutionCost = u32;

pub type Price = Ratio<u128>;

impl Side<Price> {
    /// Compare prices on opposite sides.
    pub fn overlaps(self, that: Price) -> bool {
        match self {
            // Bid price must be higher than Ask price to overlap.
            Side::Bid(this) => this >= that,
            // Ask price must be lower than Bid side to overlap.
            Side::Ask(this) => this <= that,
        }
    }

    /// Compare prices on the same side.
    pub fn better_than(self, that: Price) -> bool {
        match self {
            // If we compare Bid prices, then we favor highest price.
            Side::Bid(this) => this >= that,
            // If we compare Ask prices, then we favor lowest price.
            Side::Ask(this) => this <= that,
        }
    }
}

pub type BatcherFeePerQuote = Ratio<u64>;

pub type LPFee = Ratio<u64>;
