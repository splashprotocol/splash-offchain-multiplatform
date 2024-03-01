use derive_more::{From, Into};

use crate::FarmId;

#[derive(Copy, Clone, Into, From, Debug)]
pub struct VotingOrderId([u8; 32]);

#[derive(Clone, Debug)]
pub struct VotingOrder {
    pub id: VotingOrderId,
    pub distribution: Vec<(FarmId, u64)>,
    pub proof: Vec<u8>,
}
