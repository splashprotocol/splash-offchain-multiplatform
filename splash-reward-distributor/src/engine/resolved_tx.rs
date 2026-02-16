use cml_chain::{
    auxdata::AuxiliaryData,
    builders::{tx_builder::SignedTxBuilder, witness_builder::TransactionWitnessSetBuilder},
    transaction::{TransactionBody, TransactionOutput},
};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::OutputRef;
use splash_dao_offchain::routines::Slot;
use splash_yf_offchain::Epoch;

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

pub type PartiallySignedCardanoTx = PartiallySignedTx<SerializableSignedTxBuilder, CardanoTxInputs>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableSignedTxBuilder {
    pub body_cbor_bytes: Vec<u8>,
    pub witness_set: TransactionWitnessSetBuilder,
    pub is_valid: bool,
    pub auxiliary_data: Option<AuxiliaryData>,
}

impl From<SerializableSignedTxBuilder> for SignedTxBuilder {
    fn from(value: SerializableSignedTxBuilder) -> Self {
        use cml_chain::Deserialize;
        let tx_body = TransactionBody::from_cbor_bytes(&value.body_cbor_bytes).unwrap();
        if let Some(auxiliary_data) = value.auxiliary_data {
            SignedTxBuilder::new_with_data(tx_body, value.witness_set, value.is_valid, auxiliary_data)
        } else {
            SignedTxBuilder::new_without_data(tx_body, value.witness_set, value.is_valid)
        }
    }
}
