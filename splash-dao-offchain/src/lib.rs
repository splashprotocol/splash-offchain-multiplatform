use crate::constants::EPOCH_LEN;
use crate::network_time::NetworkTime;

mod coin;
mod constants;
mod entities;
pub mod event_sink;
mod network_time;
mod routine;
mod routines;

pub type FarmId = u64;
pub type Epoch = u64;

pub fn epoch_start(zeros_epoch_start: NetworkTime, epoch: Epoch) -> NetworkTime {
    zeros_epoch_start + epoch * EPOCH_LEN
}
