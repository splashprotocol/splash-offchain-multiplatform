use cml_chain::certs::StakeCredential;
use cml_chain::plutus::PlutusData;
use cml_crypto::RawBytesEncoding;
use cml_crypto::ScriptHash;
use serde::Deserialize;
use serde::Serialize;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::backlog::data::OrderWeight;
use spectrum_offchain::backlog::data::Weighted;
use spectrum_offchain::domain::order::PendingOrder;
use spectrum_offchain::domain::order::ProgressingOrder;
use spectrum_offchain::domain::order::UniqueOrder;
use voting_order::VotingOrder;

use super::onchain::voting_escrow::VotingEscrowId;
pub mod voting_order;

/// The id for off-chain order to extend/redeem voting escrow.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffChainOrderId {
    pub voting_escrow_id: VotingEscrowId,
    /// Current version of voting_escrow that this order will apply to.
    pub version: u64,
}

impl From<OffChainOrderId> for VotingEscrowId {
    fn from(value: OffChainOrderId) -> Self {
        value.voting_escrow_id
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum OffChainOrder {
    Extend {
        order: ExtendVotingEscrowOffChainOrder,
        timestamp: i64,
    },
    Redeem {
        order: RedeemVotingEscrowOffChainOrder,
        timestamp: i64,
    },
    Vote {
        order: VotingOrder,
        timestamp: i64,
    },
}

impl OffChainOrder {
    pub fn get_timestamp(&self) -> i64 {
        match self {
            OffChainOrder::Extend { timestamp, .. }
            | OffChainOrder::Redeem { timestamp, .. }
            | OffChainOrder::Vote { timestamp, .. } => *timestamp,
        }
    }
}

impl From<OffChainOrder> for ProgressingOrder<OffChainOrder> {
    fn from(value: OffChainOrder) -> Self {
        let timestamp = value.get_timestamp();
        ProgressingOrder {
            order: value,
            timestamp,
        }
    }
}

impl From<OffChainOrder> for PendingOrder<OffChainOrder> {
    fn from(value: OffChainOrder) -> Self {
        let timestamp = value.get_timestamp();
        PendingOrder {
            order: value,
            timestamp,
        }
    }
}

impl OffChainOrder {
    pub fn get_order_id(&self) -> OffChainOrderId {
        match self {
            OffChainOrder::Extend { order, .. } => order.id,
            OffChainOrder::Redeem { order, .. } => order.id,
            OffChainOrder::Vote { order, .. } => order.id,
        }
    }

    pub fn order_type_str(&self) -> &str {
        match self {
            OffChainOrder::Extend { .. } => "Extend",
            OffChainOrder::Redeem { .. } => "Redeem",
            OffChainOrder::Vote { .. } => "Vote",
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendVotingEscrowOffChainOrder {
    /// Refers to id of `voting_escrow` in the TX input.
    pub id: OffChainOrderId,
    pub proof: Vec<u8>,
    pub witness: ScriptHash,
    pub witness_input: String,
    pub order_output_ref: OutputRef,
}

impl UniqueOrder for OffChainOrder {
    type TOrderId = OffChainOrderId;

    fn get_self_ref(&self) -> Self::TOrderId {
        match self {
            OffChainOrder::Extend { order, .. } => order.id,
            OffChainOrder::Redeem { order, .. } => order.id,
            OffChainOrder::Vote { order, .. } => order.id,
        }
    }
}

impl Weighted for OffChainOrder {
    fn weight(&self) -> OrderWeight {
        OrderWeight::from(1)
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedeemVotingEscrowOffChainOrder {
    /// Refers to id of `voting_escrow` in the TX input.
    pub id: OffChainOrderId,
    pub stake_credential: Option<StakeCredential>,
    pub proof: Vec<u8>,
    pub witness: ScriptHash,
    pub witness_input: String,
}

pub fn compute_voting_escrow_witness_message(
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
