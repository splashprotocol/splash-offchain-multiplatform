use derive_more::{From, Into};

use crate::time::NetworkTime;

mod assets;
pub mod constants;
pub mod deployment;
pub mod entities;
mod protocol_config;
mod routine;
pub mod routines;
pub mod state_projection;
pub mod time;

#[derive(Copy, Clone, Eq, PartialEq, From, Into, Debug)]
pub struct GenesisEpochStartTime(NetworkTime);

#[derive(Copy, Clone, Eq, PartialEq, From, Into, Debug)]
pub struct CurrentEpoch(u32);
