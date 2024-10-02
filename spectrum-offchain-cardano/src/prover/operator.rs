use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::crypto::utils::make_vkey_witness;
use cml_chain::transaction::Transaction;
use cml_crypto::{Bip32PrivateKey, PrivateKey};
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::transaction::OutboundTransaction;
use spectrum_offchain::tx_prover::TxProver;

/// Signs transactions on behalf of operator.
#[derive(Clone)]
pub struct OperatorProver(String);

impl OperatorProver {
    pub fn new(sk_bech32: String) -> Self {
        Self(sk_bech32)
    }
}

impl<'a> TxProver<SignedTxBuilder, OutboundTransaction<Transaction>> for OperatorProver {
    fn prove(&self, mut candidate: SignedTxBuilder) -> OutboundTransaction<Transaction> {
        let body = candidate.body();
        let tx_hash = hash_transaction_canonical(&body);
        let sk = PrivateKey::from_bech32(self.0.as_str()).unwrap();
        let signature = make_vkey_witness(&tx_hash, &sk);
        candidate.add_vkey(signature);
        match candidate.build_checked() {
            Ok(tx) => tx.into(),
            Err(err) => panic!("CML returned error: {}", err),
        }
    }
}
