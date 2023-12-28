use cml_chain::address::Address;
use derive_more::{From, Into};

#[derive(serde::Deserialize, Debug, Clone, Into, From)]
pub struct RewardAddress(pub Address);
