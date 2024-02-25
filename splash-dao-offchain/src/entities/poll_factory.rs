use std::collections::BTreeSet;

use crate::FarmId;

pub struct PollFactory {
    pub last_poll_epoch: u64,
    pub active_farms: BTreeSet<FarmId>,
}
