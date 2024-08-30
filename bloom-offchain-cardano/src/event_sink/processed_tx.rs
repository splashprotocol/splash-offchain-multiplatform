use cml_chain::transaction::{Transaction, TransactionInput, TransactionOutput};
use cml_crypto::TransactionHash;
use spectrum_cardano_lib::hash::hash_transaction_canonical;

/// A Tx being processed.
/// Outputs in [Transaction] may be partially consumed in the process
/// while this structure preserves stable hash.
pub struct ProcessedTransaction {
    pub hash: TransactionHash,
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<(usize, TransactionOutput)>,
}

impl From<Transaction> for ProcessedTransaction {
    fn from(tx: Transaction) -> Self {
        Self {
            hash: hash_transaction_canonical(&tx.body),
            inputs: tx.body.inputs.into(),
            outputs: tx.body.outputs.into_iter().enumerate().collect(),
        }
    }
}
