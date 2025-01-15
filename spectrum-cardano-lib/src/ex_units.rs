use algebra_core::monoid::Monoid;
use algebra_core::semigroup::Semigroup;
use derive_more::{Add, AddAssign, Sub, SubAssign};

#[derive(
    serde::Serialize,
    serde::Deserialize,
    Debug,
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Add,
    Sub,
    AddAssign,
    SubAssign,
)]
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

impl Semigroup for ExUnits {
    fn combine(self, other: Self) -> Self {
        self.add(other)
    }
}

impl Monoid for ExUnits {
    fn empty() -> Self {
        ExUnits { mem: 0, steps: 0 }
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

impl From<&cml_chain::plutus::ExUnits> for ExUnits {
    fn from(value: &cml_chain::plutus::ExUnits) -> Self {
        Self {
            mem: value.mem,
            steps: value.steps,
        }
    }
}
