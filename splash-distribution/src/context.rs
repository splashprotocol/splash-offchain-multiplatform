use cml_chain::address::Address;
use cml_crypto::Ed25519KeyHash;

pub struct DistributionContext {

}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct DistributorCreds(pub Ed25519KeyHash, pub Address);
