use cml_core::serialization::RawBytesEncoding;
use cml_crypto::PublicKey;
use derive_more::{From, Into};
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::small_set::SmallVec;

#[derive(Debug, Copy, Clone, Into, From, Default)]
pub struct ConsumedInputs(pub SmallVec<OutputRef>);

#[derive(Debug, Copy, Clone, Into, From)]
pub struct ConsumedIdentifiers<I: Copy>(pub SmallVec<I>);

impl<I: Copy> Default for ConsumedIdentifiers<I> {
    fn default() -> Self {
        Self(SmallVec::default())
    }
}

#[derive(Debug, Copy, Clone, Into, From)]
pub struct ProducedIdentifiers<I: Copy>(pub SmallVec<I>);

impl<I: Copy> Default for ProducedIdentifiers<I> {
    fn default() -> Self {
        Self(SmallVec::default())
    }
}

#[derive(Debug, Copy, Clone, From, Into)]
pub struct AuthVerificationKey([u8; 32]);
impl AuthVerificationKey {
    pub fn from_bytes(pk_bytes: [u8; 32]) -> Self {
        AuthVerificationKey(pk_bytes)
    }
    pub fn get_verification_key(&self) -> PublicKey {
        PublicKey::from_raw_bytes(&self.0).unwrap()
    }
}

impl<'de> Deserialize<'de> for AuthVerificationKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)
            .and_then(|bech32_encoded_key| {
                PublicKey::from_bech32(bech32_encoded_key.as_str())
                    .map_err(|_| Error::custom(format!("Couldn't read public key {}", bech32_encoded_key)))
            })
            .and_then(|key| {
                key.to_raw_bytes().try_into().map_err(|_| {
                    Error::custom(format!(
                        "Key length should be equals to 32 bytes. Current length {}",
                        key.to_raw_bytes().len()
                    ))
                })
            })
            .map(|bytes| AuthVerificationKey(bytes))
    }
}
