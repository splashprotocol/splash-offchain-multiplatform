use cml_chain::transaction::{DatumOption, TransactionOutput};

pub trait TransactionOutputExtension {
    fn into_datum(self) -> Option<DatumOption>;
}

impl TransactionOutputExtension for TransactionOutput {
    fn into_datum(self) -> Option<DatumOption> {
        match self {
            Self::ShelleyTxOut(_) => None,
            Self::AlonzoTxOut(tx_out) => Some(DatumOption::new_hash(tx_out.datum_hash)),
            Self::BabbageTxOut(tx_out) => tx_out.datum_option,
        }
    }
}
