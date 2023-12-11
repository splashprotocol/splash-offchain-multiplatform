use cml_chain::transaction::TransactionOutput;

use crate::OutputRef;

pub struct FinalizedTxOut(pub TransactionOutput, pub OutputRef);
