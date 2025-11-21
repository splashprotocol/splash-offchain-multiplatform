use cml_core::Slot;

pub mod entities;
pub mod events;
pub mod settings;

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
    pub const FIRST: Epoch = Epoch(0);

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
    pub fn first_slot(&self, slots_in_epoch: u64, epoch_start: Slot) -> Slot {
        epoch_start + (self.0 * slots_in_epoch)
    }
    pub fn last_slot(&self, slots_in_epoch: u64, epoch_start: Slot) -> Slot {
        self.first_slot(slots_in_epoch, epoch_start) + slots_in_epoch - 1
    }

    pub fn adjacent_epochs(&self, current_epoch: Self) -> Vec<Self> {
        if self == &current_epoch {
            vec![]
        } else {
            (self.next().unwrap()..=current_epoch.unwrap())
                .map(Self::from)
                .collect()
        }
    }
}
