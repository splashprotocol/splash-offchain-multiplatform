use cml_chain::address::Address;
use derive_more::{From, Into};

#[derive(Debug, Clone, Into, From)]
pub struct OperatorAddress(pub Address);
