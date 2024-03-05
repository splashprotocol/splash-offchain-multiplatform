use cml_chain::PolicyId;
use cml_crypto::ScriptHash;
use derive_more::{From, Into};

use spectrum_offchain::data::order::UniqueOrder;

use crate::entities::onchain::smart_farm::FarmId;
use crate::entities::onchain::voting_escrow::VotingEscrowId;

#[derive(Copy, Clone, Eq, PartialEq, Hash, Into, From, Debug)]
pub struct VotingOrderId(VotingEscrowId, u64);

impl From<VotingOrderId> for VotingEscrowId {
    fn from(value: VotingOrderId) -> Self {
        value.0
    }
}

#[derive(Clone, Debug)]
pub struct VotingOrder {
    pub id: VotingOrderId,
    pub distribution: Vec<(FarmId, u64)>,
    pub proof: Vec<u8>,
    pub witness: ScriptHash,
    pub version: u32,
    pub proposal_auth_policy: PolicyId,
}

impl UniqueOrder for VotingOrder {
    type TOrderId = VotingOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}

impl UniqueOrder for VotingOrder {
    type TOrderId = VotingOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}
