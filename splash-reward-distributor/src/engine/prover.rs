use crate::engine::resolved_tx::PartiallySignedCardanoTx;
use cml_chain::transaction::Transaction;
use spectrum_offchain::tx_prover::TxProver;

pub struct VerifierProver;

impl TxProver<PartiallySignedCardanoTx, Transaction> for VerifierProver {
    fn prove(&self, candidate: PartiallySignedCardanoTx) -> Transaction {
        todo!()
    }
}
