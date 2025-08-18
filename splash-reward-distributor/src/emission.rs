use cml_core::Slot;
use serde::Deserialize;

pub trait Emission {
    fn total_emission_between(&self, a: Slot, b: Slot) -> u64;
}

pub fn reward_amount(share_bps: u64, interval_emission: u64) -> u64 {
    (share_bps * interval_emission) / 10_000
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct EmissionConfig {}

impl Emission for EmissionConfig {
    fn total_emission_between(&self, a: Slot, b: Slot) -> u64 {
        todo!("DEX-915")
    }
}
