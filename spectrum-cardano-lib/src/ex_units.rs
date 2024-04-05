use std::ops::Add;
use derive_more::{Add, AddAssign, Sub, SubAssign};
use algebra_core::monoid::Monoid;

#[derive(serde::Deserialize)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Add, Sub, AddAssign, SubAssign)]
pub struct ExUnits {
    pub mem: u64,
    pub steps: u64,
}

impl ExUnits {
    pub fn scale(self, factor: u64) -> Self {
        Self {
            mem: self.mem * factor,
            steps: self.steps * factor,
        }
    }
}

impl Monoid for ExUnits {
    fn identity() -> Self {
        ExUnits {
            mem: 0,
            steps: 0,
        }
    }
    fn combine(self, other: Self) -> Self {
        self.add(other)
    }
}

impl From<ExUnits> for cml_chain::plutus::ExUnits {
    fn from(value: ExUnits) -> Self {
        Self {
            mem: value.mem,
            steps: value.steps,
            encodings: None,
        }
    }
}
