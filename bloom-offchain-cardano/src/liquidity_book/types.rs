use std::fmt::{Debug, Formatter};

use num_rational::Ratio;
use rand::{RngCore, thread_rng};

use crate::liquidity_book::side::Side;

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct SourceId([u8; 32]);

impl SourceId {
    #[cfg(test)]
    pub fn random() -> SourceId {
        let mut bf = [0u8;32];
        thread_rng().fill_bytes(&mut bf);
        SourceId(bf)
    }
}

impl Debug for SourceId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&*hex::encode(&self.0))
    }
}

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
