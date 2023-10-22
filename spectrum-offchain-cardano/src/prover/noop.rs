use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::transaction::Transaction;

use spectrum_offchain::tx_prover::TxProver;

pub struct NoopProver {}

impl TxProver<SignedTxBuilder, Transaction> for NoopProver {
    fn prove(&self, candidate: SignedTxBuilder) -> Transaction {
        candidate.build_unchecked()
    }
}
