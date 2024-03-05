use derive_more::{From, Into};

use crate::time::NetworkTime;

pub mod constants;
pub mod entities;
mod protocol_config;
mod routine;
pub mod routines;
pub mod state_projection;
pub mod time;

#[derive(Copy, Clone, Eq, PartialEq, From, Into)]
pub struct GenesisEpochStartTime(NetworkTime);
