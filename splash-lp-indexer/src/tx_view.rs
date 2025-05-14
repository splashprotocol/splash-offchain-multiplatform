use cml_chain::transaction::{ConwayFormatTxOut, Transaction, TransactionInput, TransactionOutput};
use cml_core::Slot;
use cml_crypto::{Ed25519KeyHash, TransactionHash};
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use log::info;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::persistent_index::PersistentIndex;

/// A Tx view giving access to its mandatory fields, inputs are partially resolved.
#[derive(Debug, Clone)]
pub struct TxView {
    pub hash: TransactionHash,
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
    pub signers: Vec<Ed25519KeyHash>,
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
                signers: tx
                    .witness_set
                    .vkeywitnesses
                    .into_iter()
                    .flat_map(|witnesses| witnesses.into_iter().map(|witness| witness.vkey.hash()))
                    .collect(),
            },
            Either::Right(tx) => Self {
                hash: hash_transaction_canonical(&tx.body),
                inputs: tx.body.inputs.into(),
                outputs: tx.body.outputs,
                signers: tx
                    .witness_set
                    .vkeywitnesses
                    .into_iter()
                    .flat_map(|witnesses| witnesses.into_iter().map(|witness| witness.vkey.hash()))
                    .collect(),
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
    pub signers: Vec<Ed25519KeyHash>,
    pub slot: u64,
}

impl TxViewPartiallyResolved {
    pub async fn resolve<Index: PersistentIndex<OutputRef, TransactionOutput>>(
        tx: TxView,
        index: &Index,
        slot: Slot,
    ) -> Self {
        Self {
            hash: tx.hash,
            inputs: try_resolve_inputs(tx.inputs, index).await,
            outputs: tx.outputs,
            signers: tx.signers,
            slot,
        }
    }
}

async fn try_resolve_inputs<Index: PersistentIndex<OutputRef, TransactionOutput>>(
    inputs: Vec<TransactionInput>,
    index: &Index,
) -> Vec<(TransactionInput, Option<TransactionOutput>)> {
    // info!("Trying to resolve inputs {:?}", inputs.iter().map(|i| {
    //     format!("input hash {}, idx: {}", i.transaction_id.to_hex(), i.index)
    // }));
    let mut processed_inputs = vec![];
    for input in inputs {
        //info!("Trying to resolve input: {}#{}", input.transaction_id.to_hex(), input.index);
        let maybe_output = index.get(OutputRef::new(input.transaction_id, input.index)).await;
        //info!("Resolving result for {}#{} is {}", input.transaction_id.to_hex(), input.index, maybe_output.is_some());
        processed_inputs.push((input, maybe_output));
    }
    processed_inputs
}
