use derive_more::{From, Into};
use num_rational::Ratio;
use serde::{Deserialize, Serialize};

#[derive(Debug, Eq, PartialEq, Hash, Clone, Serialize, Deserialize)]
pub struct BacklogOrder<TOrd> {
    pub order: TOrd,
    pub timestamp: i64,
}

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Hash, Copy, Clone, Into, From, Serialize, Deserialize)]
pub struct OrderWeight(Ratio<u128>);

impl From<u64> for OrderWeight {
    fn from(x: u64) -> Self {
        Self(Ratio::from_integer(x as u128))
    }
}

impl From<Ratio<u64>> for OrderWeight {
    fn from(x: Ratio<u64>) -> Self {
        Self(Ratio::new(x.numer().clone() as u128, x.denom().clone() as u128))
    }
}

pub trait Weighted {
    fn weight(&self) -> OrderWeight;
}
