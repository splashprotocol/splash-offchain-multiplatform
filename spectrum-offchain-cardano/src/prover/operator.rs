use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::crypto::utils::make_vkey_witness;
use cml_chain::transaction::Transaction;
use cml_crypto::PrivateKey;
use log::info;

use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_offchain::tx_prover::TxProver;

/// Signs transactions on behalf of operator.
#[derive(Copy, Clone)]
pub struct OperatorProver<'a>(&'a PrivateKey);

impl<'a> OperatorProver<'a> {
    pub fn new(sk: &'a PrivateKey) -> Self {
        Self(sk)
    }
}

impl<'a> TxProver<SignedTxBuilder, Transaction> for OperatorProver<'a> {
    fn prove(&self, mut candidate: SignedTxBuilder) -> Transaction {
        let body = candidate.body();
        let signature = make_vkey_witness(&hash_transaction_canonical(&body), self.0);
        candidate.add_vkey(signature);
        candidate.build_unchecked()
    }
}
