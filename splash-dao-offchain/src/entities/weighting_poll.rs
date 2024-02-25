use crate::{Epoch, FarmId};

pub struct WeightingPoll {
    pub epoch: Epoch,
    pub distribution: Vec<(FarmId, u64)>,
}
