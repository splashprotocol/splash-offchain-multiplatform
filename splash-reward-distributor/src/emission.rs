use cml_core::Slot;

pub trait Emission {
    fn total_emission_between(&self, a: Slot, b: Slot) -> u64;
}
