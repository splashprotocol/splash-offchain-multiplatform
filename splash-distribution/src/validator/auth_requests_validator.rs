use cml_chain::crypto::Vkeywitness;
use cml_chain::transaction::TransactionBody;
use cml_crypto::Bip32PrivateKey;
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::prover::operator::OperatorProver;

pub struct AuthRequestsValidator {
    prover: OperatorProver
}

impl AuthRequestsValidator {

    pub fn new(validator_pk: String) -> Self {
        let operator_sk = Bip32PrivateKey::from_bech32(validator_pk.as_str())
            .unwrap()
            .to_raw_key();
        let sk_bech32 = operator_sk.to_bech32();
        let prover = OperatorProver::new(sk_bech32);
        Self {
            prover
        }
    }

    pub fn validate(&self, tx_to_validate: TransactionBody) -> Option<Vkeywitness> {
        // todo: add validation rules
        Some(self.prover.add_signature(tx_to_validate))
    }
}
