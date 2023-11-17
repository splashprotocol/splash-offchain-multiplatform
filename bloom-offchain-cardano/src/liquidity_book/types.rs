use num_rational::Ratio;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Base;

#[derive(Debug, Copy, Clone)]
pub struct Quote;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct InclusionCost(u32);

pub type Price = Ratio<u64>;

pub type BatcherFeePerQuote = Ratio<u64>;

pub type LPFee = Ratio<u64>;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct SourceId([u8; 32]);

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Side {
    Bid,
    Ask
}

impl Side {
    pub fn overlaps(&self, this: Price, that: Price) -> bool {
        if matches!(self, Side::Ask) { this <= that } else { this >= that }
    }
}
