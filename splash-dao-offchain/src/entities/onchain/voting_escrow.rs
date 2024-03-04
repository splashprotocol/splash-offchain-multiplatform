use std::time::Duration;

use spectrum_offchain::data::{EntitySnapshot, Stable};

use crate::time::NetworkTime;

#[derive(Copy, Clone, Debug)]
pub struct VotingEscrow {
    pub gov_token_amount: u64,
    pub locked_until: Lock,
}

impl Stable for VotingEscrow {
    type StableId = u64;
    fn stable_id(&self) -> Self::StableId {
        todo!()
    }
}

impl EntitySnapshot for VotingEscrow {
    type Version = u64;
    fn version(&self) -> Self::Version {
        todo!()
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Lock {
    Def(NetworkTime),
    Indef(Duration),
}
