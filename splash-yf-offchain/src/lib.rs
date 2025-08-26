use cml_core::Slot;

pub mod entities;
pub mod events;
pub mod settings;
pub mod ve_config;

#[derive(
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    derive_more::From,
    derive_more::Into,
    derive_more::Display,
)]
pub struct Epoch(u64);

impl Epoch {
    /// @panic if slots_in_epoch is 0
    pub fn unsafe_from_slot(slot: u64, slots_in_epoch: u64, epoch_start: Slot) -> Self {
        Self((slot - epoch_start) / slots_in_epoch)
    }
    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }
    pub fn unwrap(self) -> u64 {
        self.0
    }
    pub fn fist_slot(&self, slots_in_epoch: u64, epoch_start: Slot) -> Slot {
        epoch_start + (self.0 * slots_in_epoch)
    }
    pub fn last_slot(&self, slots_in_epoch: u64, epoch_start: Slot) -> Slot {
        self.fist_slot(slots_in_epoch, epoch_start) + slots_in_epoch - 1
    }
    pub fn adjacent_epochs(&self, slots_in_epoch: u64, epoch_start: Slot) -> Vec<Self> {
        let last_slot = self.last_slot(slots_in_epoch, epoch_start);
        let mut result = Vec::new();
        for slot in self.fist_slot(slots_in_epoch, epoch_start)..=last_slot {
            result.push(Self::unsafe_from_slot(slot, slots_in_epoch, epoch_start));
        }
        result
    }
}
