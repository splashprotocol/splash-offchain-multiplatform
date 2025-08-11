use cml_core::Slot;

pub trait Emission {
    fn total_emission_between(&self, a: Slot, b: Slot) -> u64;
}

pub fn reward_amount(share_bps: u64, interval_emission: u64) -> u64 {
    (share_bps * interval_emission) / 10_000
}
