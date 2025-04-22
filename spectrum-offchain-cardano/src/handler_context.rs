use cml_chain::assets::Mint;
use cml_chain::PolicyId;
use cml_core::serialization::RawBytesEncoding;
use cml_crypto::{Ed25519KeyHash, PublicKey};
use cml_multi_era::babbage::utils::BabbageMint;
use derive_more::{From, Into};
use log::trace;
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use spectrum_cardano_lib::{AssetName, OutputRef, Token};
use spectrum_offchain::data::small_vec::SmallVec;

#[derive(Debug, Copy, Clone, Into, From, Default)]
pub struct Mints(pub SmallVec<Token>);

impl Mints {
    pub fn contains_mint(&self, pol: PolicyId) -> bool {
        self.0.exists(|token| token.0 == pol)
    }
}

impl From<Mint> for Mints {
    fn from(mint: Mint) -> Self {
        let assets = mint.iter().flat_map(move |(pol, v)| {
            v.iter()
                .map(move |(tn, _)| Token(*pol, AssetName::from(tn.clone())))
        });
        Self(SmallVec::new(assets))
    }
}

impl From<BabbageMint> for Mints {
    fn from(value: BabbageMint) -> Self {
        let assets = value
            .assets
            .into_iter()
            .flat_map(move |(pol, v)| v.into_iter().map(move |(tn, _)| Token(pol, AssetName::from(tn))));
        Self(SmallVec::new(assets))
    }
}

#[derive(Debug, Copy, Clone, Into, From, Default)]
pub struct AllowedAdditionalPaymentDestinations(pub SmallVec<Ed25519KeyHash>);

/// PubKey hashes that didn't appear in TX inputs
#[derive(Debug, Copy, Clone, Into, From, Default)]
pub struct AddedPaymentDestinations(pub SmallVec<Ed25519KeyHash>);
impl AddedPaymentDestinations {
    pub fn complies_with(&self, whitelist: &AllowedAdditionalPaymentDestinations) -> bool {
        let is_compliant = !self.0.exists(|hash| !whitelist.0.contains(hash));
        if !is_compliant {
            trace!("AddedPaymentDestinations: {}", self.0);
            trace!("AllowedAdditionalPaymentDestinations: {}", whitelist.0);
        }
        is_compliant
    }
}

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

#[cfg(test)]
mod tests {
    use super::{AddedPaymentDestinations, AllowedAdditionalPaymentDestinations, Ed25519KeyHash};
    use spectrum_offchain::data::small_vec::SmallVec;

    #[test]
    fn test_complies_with() {
        let whitelist = AllowedAdditionalPaymentDestinations(SmallVec::new(
            vec![Ed25519KeyHash::from([1u8; 28]), Ed25519KeyHash::from([2u8; 28])].into_iter(),
        ));
        let added_destinations =
            AddedPaymentDestinations(SmallVec::new(vec![Ed25519KeyHash::from([1u8; 28])].into_iter()));

        assert!(added_destinations.complies_with(&whitelist));

        let non_compliant_destinations =
            AddedPaymentDestinations(SmallVec::new(vec![Ed25519KeyHash::from([3u8; 28])].into_iter()));

        assert!(!non_compliant_destinations.complies_with(&whitelist));
    }
}
