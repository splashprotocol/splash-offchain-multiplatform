use cml_core::serialization::Serialize;
use cml_crypto::{blake2b256, TransactionHash};

pub fn hash_transaction_canonical<TxBody: Serialize>(tx_body: &TxBody) -> TransactionHash {
    TransactionHash::from(blake2b256(tx_body.to_cbor_bytes().as_ref()))
}
