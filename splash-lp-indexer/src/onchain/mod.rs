use std::fmt::Display;

pub mod event;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct GaugeWeight(pub u64, pub u64);

impl GaugeWeight {
    pub fn mul(self, other: u64) -> u64 {
        (self.0 as u128 * other as u128 / self.1 as u128) as u64
    }
    pub fn non_zero(&self) -> bool {
        self.0 != 0
    }
}

impl Display for GaugeWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.0, self.1)
    }
}
