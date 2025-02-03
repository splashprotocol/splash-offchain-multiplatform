use cml_crypto::ScriptHash;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::{
    backlog::data::{OrderWeight, Weighted},
    domain::order::UniqueOrder,
};

use crate::entities::onchain::voting_escrow::VotingEscrowId;

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendVotingEscrowOrderId {
    pub voting_escrow_id: VotingEscrowId,
    /// Current version of voting_escrow that this order will apply to.
    pub version: u64,
}

impl From<ExtendVotingEscrowOrderId> for VotingEscrowId {
    fn from(value: ExtendVotingEscrowOrderId) -> Self {
        value.voting_escrow_id
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendVotingEscrowOffChainOrder {
    /// Refers to id of `voting_escrow` in the OUTPUT i.e. the 'extended' VE.
    pub id: ExtendVotingEscrowOrderId,
    pub proof: Vec<u8>,
    pub witness: ScriptHash,
    pub witness_input: String,
    pub order_output_ref: OutputRef,
}

impl UniqueOrder for ExtendVotingEscrowOffChainOrder {
    type TOrderId = ExtendVotingEscrowOrderId;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}

impl Weighted for ExtendVotingEscrowOffChainOrder {
    fn weight(&self) -> OrderWeight {
        OrderWeight::from(1)
    }
}
