use crate::OutputRef;
use cml_chain::transaction::TransactionOutput;
use spectrum_offchain::domain::Has;
use std::cmp::Ordering;
use type_equalities::IsEqual;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FinalizedTxOut(pub TransactionOutput, pub OutputRef);

impl Has<OutputRef> for FinalizedTxOut {
    fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.1
    }
}

impl PartialOrd for FinalizedTxOut {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(&other))
    }
}

impl Ord for FinalizedTxOut {
    fn cmp(&self, other: &Self) -> Ordering {
        self.1.cmp(&other.1)
    }
}

impl FinalizedTxOut {
    pub fn reference(&self) -> OutputRef {
        self.1
    }
}

impl FinalizedTxOut {
    pub fn new(out: TransactionOutput, out_ref: OutputRef) -> Self {
        Self(out, out_ref)
    }
}
