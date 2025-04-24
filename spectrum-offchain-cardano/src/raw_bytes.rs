use cml_crypto::{BlockHeaderHash, ScriptHash};
use spectrum_cardano_lib::Token;

pub trait RawBytes {
    fn to_raw_bytes(self) -> Vec<u8>;
}

impl RawBytes for u64 {
    fn to_raw_bytes(self) -> Vec<u8> {
        self.to_ne_bytes().to_vec()
    }
}

impl<const N: usize> RawBytes for [u8; N] {
    fn to_raw_bytes(self) -> Vec<u8> {
        self.to_vec()
    }
}

impl RawBytes for ScriptHash {
    fn to_raw_bytes(self) -> Vec<u8> {
        cml_core::serialization::RawBytesEncoding::to_raw_bytes(&self).to_vec()
    }
}

impl RawBytes for BlockHeaderHash {
    fn to_raw_bytes(self) -> Vec<u8> {
        cml_core::serialization::RawBytesEncoding::to_raw_bytes(&self).to_vec()
    }
}

impl RawBytes for Token {
    fn to_raw_bytes(self) -> Vec<u8> {
        self.into()
    }
}
