use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use derive_more::{From, Into};
use time::NetworkTimeProvider;

use crate::time::NetworkTime;

mod assets;
pub mod constants;
pub mod deployment;
pub mod entities;
pub mod funding;
pub mod handler;
pub mod protocol_config;
mod routine;
pub mod routines;
pub mod state_projection;
pub mod time;

#[derive(Copy, Clone, Eq, PartialEq, From, Into, Debug)]
pub struct GenesisEpochStartTime(NetworkTime);

#[derive(Copy, Clone, Eq, PartialEq, From, Into, Debug)]
pub struct CurrentEpoch(u32);

pub struct NetworkTimeSource;

#[async_trait]
impl NetworkTimeProvider for NetworkTimeSource {
    async fn network_time(&self) -> NetworkTime {
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }
}
