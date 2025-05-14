use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::crypto::Vkeywitness;
use cml_chain::transaction::{Transaction, TransactionBody};

use spectrum_offchain::tx_prover::TxProver;

pub struct NoopProver {}

impl TxProver<SignedTxBuilder, Transaction> for NoopProver {
    fn prove(&self, candidate: SignedTxBuilder) -> Transaction {
        candidate.build_unchecked()
    }

    fn add_signature(&self, candidate: TransactionBody) -> Vkeywitness {
        todo!()
    }
}
