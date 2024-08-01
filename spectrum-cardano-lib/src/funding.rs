use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use derive_more::{From, Into};
#[derive(Clone, Debug, Into, From)]
pub struct OperatorFunding(TransactionUnspentOutput);
