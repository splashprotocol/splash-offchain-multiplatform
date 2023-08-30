use serde::{Deserialize, Serialize};

#[derive(Debug, Eq, PartialEq, Hash, Clone, Serialize, Deserialize)]
pub struct BacklogOrder<TOrd> {
    pub order: TOrd,
    pub timestamp: i64,
}

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Hash, Copy, Clone, Serialize, Deserialize)]
pub struct OrderWeight(u64);

impl From<u64> for OrderWeight {
    fn from(x: u64) -> Self {
        Self(x)
    }
}

pub trait Weighted {
    fn weight(&self) -> OrderWeight;
}
