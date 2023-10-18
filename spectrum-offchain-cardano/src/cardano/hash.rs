use cml_chain::transaction::TransactionBody;
use cml_core::serialization::Serialize;
use cml_crypto::{blake2b256, TransactionHash};

pub fn hash_transaction(tx_body: &TransactionBody) -> TransactionHash {
    TransactionHash::from(blake2b256(tx_body.to_cbor_bytes().as_ref()))
}
