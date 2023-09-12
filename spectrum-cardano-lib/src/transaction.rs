use cml_chain::certs::StakeCredential;
use cml_chain::transaction::{DatumOption, TransactionOutput};
use cml_chain::Value;
use cml_crypto::ScriptHash;

pub trait TransactionOutputExtension {
    fn into_datum(self) -> Option<DatumOption>;
    fn script_hash(&self) -> Option<ScriptHash>;
    fn update_value(&mut self, value: Value);
}

impl TransactionOutputExtension for TransactionOutput {
    fn into_datum(self) -> Option<DatumOption> {
        match self {
            Self::ShelleyTxOut(_) => None,
            Self::AlonzoTxOut(tx_out) => Some(DatumOption::new_hash(tx_out.datum_hash)),
            Self::BabbageTxOut(tx_out) => tx_out.datum_option,
        }
    }
    fn script_hash(&self) -> Option<ScriptHash> {
        match self.address().payment_cred()? {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(*hash),
        }
    }
    fn update_value(&mut self, value: Value) {
        match self {
            TransactionOutput::ShelleyTxOut(ref mut out) => {
                out.amount = value;
            }
            TransactionOutput::AlonzoTxOut(ref mut out) => {
                out.amount = value;
            }
            TransactionOutput::BabbageTxOut(ref mut out) => {
                out.amount = value;
            }
        }
    }
}
