use cml_chain::transaction::{TransactionInput, TransactionOutput};
use cml_crypto::TransactionHash;

/// A Tx view giving access to its mandatory fields, inputs are partially resolved.
#[derive(Debug, Clone)]
pub struct TxViewPartiallyResolved {
    pub hash: TransactionHash,
    pub inputs: Vec<(TransactionInput, Option<TransactionOutput>)>,
    pub outputs: Vec<TransactionOutput>,
}
