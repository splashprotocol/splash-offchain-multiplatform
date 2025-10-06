use crate::engine::resolved_tx::PartiallySignedCardanoTx;
use cml_chain::{crypto::utils::make_vkey_witness, transaction::Transaction};
use cml_crypto::PrivateKey;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_offchain::tx_prover::TxProver;

#[derive(Clone, derive_more::From)]
pub struct VerifierProver(String);

impl TxProver<PartiallySignedCardanoTx, Transaction> for VerifierProver {
    fn prove(&self, candidate: PartiallySignedCardanoTx) -> Transaction {
        let body = candidate.tx.body;
        let tx_hash = hash_transaction_canonical(&body);
        let sk = PrivateKey::from_bech32(self.0.as_str()).unwrap();
        let signature = make_vkey_witness(&tx_hash, &sk);

        let mut witness_set = candidate.tx.witness_set.clone();
        let Some(mut vkeys) = witness_set.vkeywitnesses else {
            panic!("No vkeys in witness set");
        };
        vkeys.push(signature);
        witness_set.vkeywitnesses = Some(vkeys);

        Transaction {
            body,
            witness_set,
            is_valid: true,
            auxiliary_data: None,
            encodings: None,
        }
    }
}
