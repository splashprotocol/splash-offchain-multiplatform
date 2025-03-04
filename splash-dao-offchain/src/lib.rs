use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use derive_more::{From, Into};
use time::NetworkTimeProvider;

use crate::time::NetworkTime;

mod assets;
pub mod collateral;
pub mod collect_utxos;
pub mod constants;
pub mod create_change_output;
pub mod deployment;
pub mod entities;
pub mod funding;
pub mod handler;
pub mod protocol_config;
mod routine;
pub mod routines;
pub mod state_projection;
pub mod time;
pub mod util;

#[derive(Copy, Clone, Eq, PartialEq, From, Into, Debug)]
pub struct GenesisEpochStartTime(NetworkTime);

#[derive(Copy, Clone, Eq, PartialEq, From, Into, Debug)]
pub struct CurrentEpoch(pub u32);

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
