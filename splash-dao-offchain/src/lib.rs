use derive_more::{From, Into};

use crate::time::NetworkTime;

mod constants;
mod entities;
pub mod event_sink;
mod routine;
mod routines;
mod time;

pub type FarmId = u64;

#[derive(Copy, Clone, Eq, PartialEq, From, Into)]
pub struct GenesisEpochStartTime(NetworkTime);
