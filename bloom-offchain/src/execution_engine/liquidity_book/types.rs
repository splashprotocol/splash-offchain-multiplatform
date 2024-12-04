use std::fmt::{Display, Formatter};
use std::str::FromStr;

use crate::execution_engine::liquidity_book::side::{OnSide, Side};
use bignumber::BigNumber;
use derive_more::{Add, Div, From, Into, Mul, Sub};
use num_rational::Ratio;
use serde::{Deserialize, Serialize};

pub type Lovelace = u64;

pub type ExCostUnits = u64;

/// Price of input asset denominated in units of output asset (Output/Input).
pub type RelativePrice = Ratio<u128>;

pub type InputAsset<T> = T;
pub type OutputAsset<T> = T;
pub type FeeAsset<T> = T;

/// Price of base asset denominated in units of quote asset (Quote/Base).
#[repr(transparent)]
#[derive(
    Debug,
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Div,
    Mul,
    Sub,
    Add,
    From,
    Into,
    Serialize,
    Deserialize,
)]
pub struct AbsolutePrice(Ratio<u128>);

impl AbsolutePrice {
    pub fn to_signed(self) -> Ratio<i128> {
        let r = self.0;
        Ratio::new(*r.numer() as i128, *r.denom() as i128)
    }
}

impl Display for AbsolutePrice {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let price = BigNumber::from_str(self.0.numer().to_string().as_str())
            .unwrap()
            .div(BigNumber::from_str(self.0.denom().to_string().as_str()).unwrap());
        f.write_str(&*format!(
            "AbsPrice(decimal={}, ratio={})",
            price.to_precision(5).to_string(),
            self.0
        ))
    }
}

impl AbsolutePrice {
    #[inline]
    pub fn new_unsafe(numer: u64, denom: u64) -> AbsolutePrice {
        Self(Ratio::new_raw(numer as u128, denom as u128))
    }
    #[inline]
    pub fn new_raw(numer: u128, denom: u128) -> AbsolutePrice {
        Self(Ratio::new_raw(numer, denom))
    }

    #[inline]
    pub fn new(numer: u64, denom: u64) -> Option<AbsolutePrice> {
        if denom != 0 {
            Some(Self(Ratio::new_raw(numer as u128, denom as u128)))
        } else {
            None
        }
    }

    #[inline]
    pub fn zero() -> AbsolutePrice {
        Self::new_unsafe(0, 1)
    }

    pub fn from_price(side: Side, price: RelativePrice) -> Self {
        Self(match side {
            // In case of bid the price in order is base/quote, so we inverse it.
            Side::Bid => price.pow(-1),
            Side::Ask => price,
        })
    }

    #[inline]
    pub const fn numer(&self) -> &u128 {
        &self.0.numer()
    }

    #[inline]
    pub const fn denom(&self) -> &u128 {
        &self.0.denom()
    }

    #[inline]
    pub const fn unwrap(self) -> Ratio<u128> {
        self.0
    }
}

impl OnSide<AbsolutePrice> {
    /// Compare prices on opposite sides.
    pub fn overlaps(self, that: AbsolutePrice) -> bool {
        match self {
            // Bid price must be higher than Ask price to overlap.
            OnSide::Bid(this) => this >= that,
            // Ask price must be lower than Bid side to overlap.
            OnSide::Ask(this) => this <= that,
        }
    }

    /// Compare prices on the same side.
    pub fn better_than(self, that: AbsolutePrice) -> bool {
        match self {
            // If we compare Bid prices, then we favor the highest price.
            OnSide::Bid(this) => this >= that,
            // If we compare Ask prices, then we favor the lowest price.
            OnSide::Ask(this) => this <= that,
        }
    }
}
