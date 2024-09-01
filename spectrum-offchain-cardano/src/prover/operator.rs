use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::crypto::utils::make_vkey_witness;
use cml_chain::transaction::Transaction;
use cml_core::serialization::Serialize;
use cml_crypto::PrivateKey;
use log::trace;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::transaction::OutboundTransaction;
use spectrum_offchain::tx_prover::TxProver;

/// Signs transactions on behalf of operator.
#[derive(Copy, Clone)]
pub struct OperatorProver<'a>(&'a PrivateKey);

impl<'a> OperatorProver<'a> {
    pub fn new(sk: &'a PrivateKey) -> Self {
        Self(sk)
    }
}

impl<'a> TxProver<SignedTxBuilder, OutboundTransaction<Transaction>> for OperatorProver<'a> {
    fn prove(&self, mut candidate: SignedTxBuilder) -> OutboundTransaction<Transaction> {
        let body = candidate.body();
        trace!(
            "Transaction body CBOR to sign: {}",
            hex::encode(body.to_cbor_bytes())
        );
        trace!(
            "Transaction body hash to sign: {}",
            hash_transaction_canonical(&body).to_hex()
        );
        let signature = make_vkey_witness(&hash_transaction_canonical(&body), self.0);
        candidate.add_vkey(signature);
        candidate.build_unchecked().into()
    }
}
