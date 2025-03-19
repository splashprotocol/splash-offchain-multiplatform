use cml_chain::auxdata::Metadata;
use cml_chain::transaction::{ConwayFormatTxOut, Transaction, TransactionInput, TransactionOutput};
use cml_crypto::{Ed25519KeyHash, TransactionHash};
use cml_multi_era::babbage::{BabbageAuxiliaryData, BabbageTransaction};
use either::Either;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_offchain_cardano::handler_context::Mints;

#[derive(Clone)]
/// A Tx being processed.
/// Outputs in [Transaction] may be partially consumed in the process
/// while this structure preserves stable hash.
pub struct TxViewMut {
    pub hash: TransactionHash,
    pub inputs: Vec<TransactionInput>,
    /// Indexed outputs
    pub outputs: Vec<(usize, TransactionOutput)>,
    pub metadata: Option<Metadata>,
    pub mints: Option<Mints>,
    pub signers: Vec<Ed25519KeyHash>,
}

impl From<Transaction> for TxViewMut {
    fn from(tx: Transaction) -> Self {
        Self {
            hash: hash_transaction_canonical(&tx.body),
            inputs: tx.body.inputs.into(),
            outputs: tx.body.outputs.into_iter().enumerate().collect(),
            metadata: tx.auxiliary_data.and_then(|md| md.metadata().cloned()),
            mints: tx.body.mint.map(|inner| inner.into()),
            signers: tx
                .witness_set
                .vkeywitnesses
                .map(|vks| vks.iter().map(|vk| vk.vkey.hash()).collect())
                .unwrap_or_else(|| vec![]),
        }
    }
}

impl From<Either<BabbageTransaction, Transaction>> for TxViewMut {
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
                mints: tx.body.mint.map(|inner| inner.into()),
                signers: tx
                    .witness_set
                    .vkeywitnesses
                    .map(|vks| vks.iter().map(|vk| vk.vkey.hash()).collect())
                    .unwrap_or_else(|| vec![]),
            },
            Either::Right(tx) => Self::from(tx),
        }
    }
}
