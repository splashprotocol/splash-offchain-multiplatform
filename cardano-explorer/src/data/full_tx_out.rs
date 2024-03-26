use cml_chain::address::Address;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionInput, TransactionOutput};
use cml_core::serialization::FromBytes;
use cml_crypto::{DatumHash, TransactionHash};
use serde::Deserialize;

use crate::data::value::ExplorerValue;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerTxOut {
    tx_hash: String,
    index: u64,
    addr: String,
    value: ExplorerValue,
    data: Option<String>,
    data_hash: Option<String>,
}

impl ExplorerTxOut {
    pub fn get_value(&self) -> &ExplorerValue {
        &self.value
    }
    pub fn try_into_cml(self) -> Option<TransactionUnspentOutput> {
        let datum = if let Some(hash) = self.data_hash {
            Some(DatumOption::new_hash(DatumHash::from_hex(hash.as_str()).ok()?))
        } else if let Some(datum) = self.data {
            Some(DatumOption::new_datum(
                PlutusData::from_bytes(hex::decode(datum).ok()?).ok()?,
            ))
        } else {
            None
        };
        let input = TransactionInput::new(TransactionHash::from_hex(self.tx_hash.as_str()).ok()?, self.index);
        let output = TransactionOutput::ConwayFormatTxOut(ConwayFormatTxOut {
            address: Address::from_bech32(self.addr.as_str()).unwrap(),
            amount: ExplorerValue::try_into(self.value).unwrap(),
            datum_option: datum,
            script_reference: None,
            encodings: None,
        });
        Some(TransactionUnspentOutput::new(input, output))
    }
}
