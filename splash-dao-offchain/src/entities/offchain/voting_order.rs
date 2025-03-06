use cml_chain::plutus::PlutusData;
use cml_crypto::{RawBytesEncoding, ScriptHash};
use derive_more::{From, Into};

use serde::{Deserialize, Serialize};
use spectrum_offchain::backlog::data::{OrderWeight, Weighted};
use spectrum_offchain::domain::order::UniqueOrder;

use crate::entities::onchain::smart_farm::FarmId;
use crate::entities::onchain::voting_escrow::VotingEscrowId;

use super::OffChainOrderId;

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct VotingOrder {
    pub id: OffChainOrderId,
    pub distribution: Vec<(FarmId, u64)>,
    pub proof: Vec<u8>,
    pub witness: ScriptHash,
    pub witness_input: String,
    pub version: u32,
    // pub proposal_auth_policy: PolicyId,
}
