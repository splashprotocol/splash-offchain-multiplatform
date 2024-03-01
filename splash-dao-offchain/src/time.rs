use async_trait::async_trait;

use crate::constants::EPOCH_LEN;
use crate::GenesisEpochStartTime;

pub type NetworkTime = u64;
pub type ProtocolEpoch = u64;

pub fn epoch_start(gen_epoch_start: GenesisEpochStartTime, epoch: ProtocolEpoch) -> NetworkTime {
    <u64>::from(gen_epoch_start) + epoch * EPOCH_LEN
}

pub fn epoch_end(gen_epoch_start: GenesisEpochStartTime, epoch: ProtocolEpoch) -> NetworkTime {
    epoch_start(gen_epoch_start, epoch) + EPOCH_LEN
}

#[async_trait]
pub trait NetworkTimeProvider {
    async fn network_time(&self) -> NetworkTime;
}

#[async_trait]
pub trait ProtocolTimeProvider {
    async fn epoch(&self) -> ProtocolEpoch;
}
