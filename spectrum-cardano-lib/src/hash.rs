use cml_core::serialization::Serialize;
use cml_crypto::{blake2b256, BlockHeaderHash, TransactionHash};
use cml_multi_era::utils::MultiEraBlockHeader;

pub fn hash_transaction_canonical<TxBody: Serialize>(tx_body: &TxBody) -> TransactionHash {
    TransactionHash::from(blake2b256(tx_body.to_cbor_bytes().as_ref()))
}

pub fn hash_block_header_canonical<Header: Serialize>(header: &Header) -> BlockHeaderHash {
    BlockHeaderHash::from(blake2b256(header.to_cbor_bytes().as_ref()))
}

pub fn hash_block_header_canonical_multi_era(header: &MultiEraBlockHeader) -> BlockHeaderHash {
    match header {
        MultiEraBlockHeader::Shelley(header) => {
            BlockHeaderHash::from(blake2b256(header.to_cbor_bytes().as_ref()))
        }
        MultiEraBlockHeader::Babbage(header) => {
            BlockHeaderHash::from(blake2b256(header.to_cbor_bytes().as_ref()))
        }
        _ => panic!("Unsupported block header type"),
    }
}
