use cml_chain::transaction::{ConwayFormatTxOut, Transaction, TransactionInput, TransactionOutput};
use cml_crypto::TransactionHash;
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::kv_store::KvStore;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A Tx view giving access to its mandatory fields, inputs are partially resolved.
#[derive(Debug, Clone)]
pub struct TxView {
    pub hash: TransactionHash,
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
}

impl From<Either<BabbageTransaction, Transaction>> for TxView {
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
                    .collect(),
            },
            Either::Right(tx) => Self {
                hash: hash_transaction_canonical(&tx.body),
                inputs: tx.body.inputs.into(),
                outputs: tx.body.outputs,
            },
        }
    }
}

/// A Tx view giving access to its mandatory fields, inputs are partially resolved.
#[derive(Debug, Clone)]
pub struct TxViewPartiallyResolved {
    pub hash: TransactionHash,
    pub inputs: Vec<(TransactionInput, Option<TransactionOutput>)>,
    pub outputs: Vec<TransactionOutput>,
}

impl TxViewPartiallyResolved {
    pub async fn resolve<Index: KvStore<OutputRef, TransactionOutput>>(
        tx: TxView,
        index: Arc<Mutex<Index>>,
    ) -> Self {
        Self {
            hash: tx.hash,
            inputs: try_resolve_inputs(tx.inputs, index.clone()).await,
            outputs: tx.outputs,
        }
    }
}

async fn try_resolve_inputs<Index: KvStore<OutputRef, TransactionOutput>>(
    inputs: Vec<TransactionInput>,
    index: Arc<Mutex<Index>>,
) -> Vec<(TransactionInput, Option<TransactionOutput>)> {
    let mut processed_inputs = vec![];
    let index = index.lock().await;
    for input in inputs {
        let maybe_output = index.get(OutputRef::new(input.transaction_id, input.index)).await;
        processed_inputs.push((input, maybe_output));
    }
    processed_inputs
}
