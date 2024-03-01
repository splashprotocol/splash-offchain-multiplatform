use crate::{FarmId, GenesisEpochStartTime};
use crate::time::{epoch_end, epoch_start, NetworkTime, ProtocolEpoch};

pub struct WeightingPoll {
    pub epoch: ProtocolEpoch,
    pub distribution: Vec<(FarmId, u64)>,
    pub reserves_splash: u64,
}

impl WeightingPoll {
    pub fn new(epoch: ProtocolEpoch, farms: Vec<FarmId>) -> Self {
        Self {
            epoch,
            distribution: farms.into_iter().map(|farm| (farm, 0)).collect(),
            reserves_splash: 0,
        }
    }

    pub fn weighting_open(&self, time_now: NetworkTime, genesis_epoch_start: GenesisEpochStartTime) -> bool {
        epoch_start(genesis_epoch_start, self.epoch) < time_now
            && epoch_end(genesis_epoch_start, self.epoch) > time_now
    }

    pub fn distribution_finished(&self) -> bool {
        self.reserves_splash == 0
    }
}
