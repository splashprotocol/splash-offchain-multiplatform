use cml_chain::transaction::TransactionInput;
use cml_crypto::TransactionHash;
use cml_multi_era::babbage::{BabbageTransaction, BabbageTransactionOutput};
use spectrum_cardano_lib::hash::hash_transaction_canonical;

/// A Tx being processed.
/// Outputs in [BabbageTransaction] may be partially consumed in the process
/// while this structure preserves stable hash.
pub struct ProcessedTransaction {
    pub hash: TransactionHash,
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<(usize, BabbageTransactionOutput)>,
}

impl From<BabbageTransaction> for ProcessedTransaction {
    fn from(tx: BabbageTransaction) -> Self {
        Self {
            hash: hash_transaction_canonical(&tx.body),
            inputs: tx.body.inputs,
            outputs: tx.body.outputs.into_iter().enumerate().collect(),
        }
    }
}
