use cml_chain::plutus::PlutusData;
use cml_crypto::{RawBytesEncoding, ScriptHash};
use derive_more::{From, Into};

use serde::{Deserialize, Serialize};
use spectrum_offchain::backlog::data::{OrderWeight, Weighted};
use spectrum_offchain::domain::order::UniqueOrder;

use crate::entities::onchain::smart_farm::FarmId;
use crate::entities::onchain::voting_escrow::VotingEscrowId;

#[derive(Copy, Clone, Eq, PartialEq, Hash, Into, From, Debug, Serialize, Deserialize)]
pub struct VotingOrderId {
    pub voting_escrow_id: VotingEscrowId,
    /// Current version of voting_escrow that this order will apply to.
    pub version: u64,
}

impl From<VotingOrderId> for VotingEscrowId {
    fn from(value: VotingOrderId) -> Self {
        value.voting_escrow_id
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct VotingOrder {
    pub id: VotingOrderId,
    pub distribution: Vec<(FarmId, u64)>,
    pub proof: Vec<u8>,
    pub witness: ScriptHash,
    pub witness_input: String,
    pub version: u32,
    // pub proposal_auth_policy: PolicyId,
}

impl UniqueOrder for VotingOrder {
    type TOrderId = VotingOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}

impl Weighted for VotingOrder {
    fn weight(&self) -> OrderWeight {
        OrderWeight::from(1)
    }
}

pub fn compute_voting_witness_message(
    witness: ScriptHash,
    witness_input: String,
    authenticated_version: u64,
) -> Result<Vec<u8>, ()> {
    use cml_chain::Serialize;
    let mut bytes = witness.to_raw_bytes().to_vec();
    let witness_input_cbor = hex::decode(witness_input).map_err(|_| ())?;
    bytes.extend_from_slice(&witness_input_cbor);
    bytes.extend_from_slice(
        &PlutusData::new_integer(cml_chain::utils::BigInteger::from(authenticated_version)).to_cbor_bytes(),
    );
    Ok(cml_crypto::blake2b256(bytes.as_ref()).to_vec())
}
