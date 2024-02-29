use cml_chain::{
    transaction::{cbor_encodings::TransactionInputEncoding, TransactionInput},
    PolicyId,
};
use cml_crypto::TransactionHash;
use rand::Rng;

pub mod babbage;
pub mod pool;
pub mod spot;

fn gen_policy_id() -> PolicyId {
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 28];
    rng.fill(&mut bytes[..]);
    PolicyId::from(bytes)
}

fn gen_transaction_input(index: u64) -> TransactionInput {
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes[..]);
    let transaction_id = TransactionHash::from(bytes);
    let encodings = Some(TransactionInputEncoding {
        len_encoding: cml_chain::LenEncoding::Canonical,
        transaction_id_encoding: cml_crypto::StringEncoding::Definite(cbor_event::Sz::One),
        index_encoding: Some(cbor_event::Sz::Inline),
    });
    TransactionInput {
        transaction_id,
        index,
        encodings,
    }
}
