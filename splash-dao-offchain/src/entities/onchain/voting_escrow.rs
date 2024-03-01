use std::time::Duration;

use crate::time::NetworkTime;

#[derive(Copy, Clone, Debug)]
pub struct VotingEscrow {
    pub gov_token_amount: u64,
    pub locked_until: Lock,
}

#[derive(Copy, Clone, Debug)]
pub enum Lock {
    Def(NetworkTime),
    Indef(Duration),
}
