use cml_chain::auxdata::Metadata;
use cml_chain::transaction::{ConwayFormatTxOut, Transaction, TransactionInput, TransactionOutput};
use cml_crypto::TransactionHash;
use cml_multi_era::babbage::{BabbageAuxiliaryData, BabbageTransaction};
use either::Either;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;

/// A Tx being processed.
/// Outputs in [Transaction] may be partially consumed in the process
/// while this structure preserves stable hash.
pub struct TxViewAtEraBoundary {
    pub hash: TransactionHash,
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<(usize, TransactionOutput)>,
    pub metadata: Option<Metadata>,
}

impl From<Transaction> for TxViewAtEraBoundary {
    fn from(tx: Transaction) -> Self {
        Self {
            hash: hash_transaction_canonical(&tx.body),
            inputs: tx.body.inputs.into(),
            outputs: tx.body.outputs.into_iter().enumerate().collect(),
            metadata: tx.auxiliary_data.and_then(|md| md.metadata().cloned()),
        }
    }
}

impl From<Either<BabbageTransaction, Transaction>> for TxViewAtEraBoundary {
    fn from(tx: Either<BabbageTransaction, Transaction>) -> Self {
        match tx {
            Either::Left(tx) => Self {
                hash: hash_transaction_canonical(&tx.body),
                inputs: tx.body.inputs.into(),
                outputs: tx
                    .body
                    .outputs
                    .into_iter()
                    .map(|out| {
                        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
                            address: out.address().clone(),
                            amount: out.value().clone(),
                            datum_option: out.datum(),
                            script_reference: None,
                            encodings: None,
                        })
                    })
                    .enumerate()
                    .collect(),
                metadata: tx.auxiliary_data.and_then(|aux_data| match aux_data {
                    BabbageAuxiliaryData::Shelley(shelley) => Some(shelley),
                    BabbageAuxiliaryData::ShelleyMA(shelley_ma) => Some(shelley_ma.transaction_metadata),
                    BabbageAuxiliaryData::Babbage(babbage) => babbage.metadata,
                }),
            },
            Either::Right(tx) => Self::from(tx),
        }
    }
}
