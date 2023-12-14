use cml_chain::builders::input_builder::InputBuilderResult;
use derive_more::{From, Into};

#[derive(Clone, Debug, Into, From)]
pub struct Collateral(pub InputBuilderResult);
