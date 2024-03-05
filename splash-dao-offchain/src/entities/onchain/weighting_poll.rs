use derive_more::From;

use spectrum_cardano_lib::Token;
use spectrum_offchain::data::{EntitySnapshot, Identifier, Stable};

use crate::entities::onchain::smart_farm::FarmId;
use crate::time::{epoch_end, epoch_start, NetworkTime, ProtocolEpoch};
use crate::{FarmId, GenesisEpochStartTime};

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd, From)]
pub struct WeightingPollId(Token);

impl Identifier for WeightingPollId {
    type For = WeightingPoll;
}

pub struct WeightingPoll {
    pub epoch: ProtocolEpoch,
    pub distribution: Vec<(FarmId, u64)>,
}

impl Stable for WeightingPoll {
    type StableId = u64;
    fn stable_id(&self) -> Self::StableId {
        todo!()
    }
}

impl EntitySnapshot for WeightingPoll {
    type Version = u64;
    fn version(&self) -> Self::Version {
        todo!()
    }
}

pub struct WeightingOngoing;
pub struct DistributionOngoing(FarmId, u64);
impl DistributionOngoing {
    pub fn farm_id(&self) -> FarmId {
        self.0
    }
    pub fn farm_weight(&self) -> u64 {
        self.1
    }
}

pub struct PollExhausted;

pub enum PollState {
    WeightingOngoing(WeightingOngoing),
    DistributionOngoing(DistributionOngoing),
    PollExhausted(PollExhausted),
}

impl WeightingPoll {
    pub fn new(epoch: ProtocolEpoch, farms: Vec<FarmId>) -> Self {
        Self {
            epoch,
            distribution: farms.into_iter().map(|farm| (farm, 0)).collect(),
        }
    }

    pub fn reserves_splash(&self) -> u64 {
        self.distribution.iter().fold(0, |acc, (_, i)| acc + *i)
    }

    pub fn next_farm(&self) -> Option<(FarmId, u64)> {
        self.distribution.iter().find(|x| x.1 > 0).copied()
    }

    pub fn state(&self, genesis: GenesisEpochStartTime, time_now: NetworkTime) -> PollState {
        if self.weighting_open(genesis, time_now) {
            PollState::WeightingOngoing(WeightingOngoing)
        } else {
            match self.next_farm() {
                None => PollState::PollExhausted(PollExhausted),
                Some((farm, weight)) => PollState::DistributionOngoing(DistributionOngoing(farm, weight)),
            }
        }
    }

    fn weighting_open(&self, genesis: GenesisEpochStartTime, time_now: NetworkTime) -> bool {
        epoch_start(genesis, self.epoch) < time_now && epoch_end(genesis, self.epoch) > time_now
    }

    fn distribution_finished(&self) -> bool {
        self.reserves_splash() == 0
    }
}
