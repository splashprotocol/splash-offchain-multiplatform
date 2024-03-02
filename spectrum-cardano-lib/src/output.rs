use cml_chain::transaction::TransactionOutput;
use cml_multi_era::babbage::BabbageTransactionOutput;
use spectrum_offchain::data::Has;
use type_equalities::IsEqual;

use crate::transaction::BabbageTransactionOutputExtension;
use crate::OutputRef;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FinalizedTxOut(pub TransactionOutput, pub OutputRef);

impl FinalizedTxOut {
    pub fn new(out: BabbageTransactionOutput, out_ref: OutputRef) -> Self {
        Self(out.upcast(), out_ref)
    }
}

impl Has<OutputRef> for FinalizedTxOut {
    fn get_labeled<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.1
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IndexedTxOut(pub usize, pub TransactionOutput);
