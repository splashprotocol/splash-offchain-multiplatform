use cml_chain::address::Address;
use cml_crypto::{Ed25519KeyHash, PrivateKey};
use derive_more::{From, Into};

#[derive(serde::Deserialize, Debug, Clone, Into, From)]
pub struct RewardAddress(pub Address);

#[derive(serde::Deserialize, Debug, Copy, Clone, Into, From)]
pub struct ExecutorCred(pub Ed25519KeyHash);
