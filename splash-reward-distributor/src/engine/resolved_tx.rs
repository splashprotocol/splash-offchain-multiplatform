use cml_chain::transaction::{Transaction, TransactionOutput};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::OutputRef;
use splash_dao_offchain::routines::Slot;

/// Transaction with resolved inputs attached to it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartiallySignedTx<Tx, Inputs> {
    pub tx: Tx,
    pub inputs: Inputs,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CardanoTxInput {
    pub output_ref: OutputRef,
    pub tx_output: TransactionOutput,
    pub issued_at: Option<Slot>,
}

pub type CardanoTxInputs = Vec<CardanoTxInput>;

pub type PartiallySignedCardanoTx = PartiallySignedTx<Transaction, CardanoTxInputs>;
