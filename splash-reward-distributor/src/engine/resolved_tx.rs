use cml_chain::transaction::{Transaction, TransactionOutput};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::OutputRef;

/// Transaction with resolved inputs attached to it.
#[derive(Clone, Serialize, Deserialize)]
pub struct PartiallySignedTx<Tx, Inputs> {
    pub tx: Tx,
    pub inputs: Inputs,
}

pub type CardanoTxInputs = Vec<(OutputRef, TransactionOutput)>;

pub type PartiallySignedCardanoTx = PartiallySignedTx<Transaction, CardanoTxInputs>;
