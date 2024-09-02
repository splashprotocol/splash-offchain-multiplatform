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
        let tx_hash = hash_transaction_canonical(&body);
        trace!("Tx body hash to sign: {}", tx_hash.to_hex());
        trace!("Tx body bytes to sign: {}", hex::encode(body.to_cbor_bytes()));
        let signature = make_vkey_witness(&tx_hash, self.0);
        candidate.add_vkey(signature);
        trace!("Tx body hash after signing: {}", hash_transaction_canonical(&candidate.body()).to_hex());
        trace!(
            "Tx body bytes after signing: {}",
            hex::encode(candidate.body().to_cbor_bytes())
        );
        let built_tx = match candidate.build_checked() {
            Ok(tx) => tx,
            Err(err) => panic!("CML returned error: {}", err),
        };
        trace!("Tx body hash after building: {}", hash_transaction_canonical(&built_tx.body).to_hex());
        trace!(
            "Tx body bytes after building: {}",
            hex::encode(built_tx.body.to_cbor_bytes())
        );
        built_tx.into()
    }
}
