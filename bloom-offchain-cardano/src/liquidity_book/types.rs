use num_rational::Ratio;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Base;

#[derive(Debug, Copy, Clone)]
pub struct Quote;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct InclusionCost(u32);

pub type ConstantPrice = Ratio<u64>;

pub type CFMMPrice = Ratio<u64>;

pub type BatcherFeePerQuote = Ratio<u64>;

pub type LPFee = Ratio<u64>;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct SourceId([u8; 32]);
