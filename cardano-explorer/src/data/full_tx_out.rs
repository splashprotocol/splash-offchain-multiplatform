use cml_chain::address::Address;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::{DatumOption, TransactionInput, TransactionOutput};
use cml_core::serialization::FromBytes;
use cml_crypto::{DatumHash, TransactionHash};
use serde::Deserialize;

use crate::data::value::ExplorerValue;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerTxOut {
    pub tx_hash: String,
    pub index: u64,
    pub addr: String,
    pub value: ExplorerValue,
    pub data: Option<String>,
    pub data_hash: Option<String>,
}

impl ExplorerTxOut {
    pub fn get_value(&self) -> &ExplorerValue {
        &self.value
    }
}

pub struct ParsingError;

impl TryInto<TransactionUnspentOutput> for ExplorerTxOut {
    type Error = ParsingError;
    fn try_into(self) -> Result<TransactionUnspentOutput, Self::Error> {
        let datum = if let Some(hash) = self.data_hash {
            Some(DatumOption::new_hash(DatumHash::from_hex(hash.as_str()).unwrap()))
        } else if let Some(datum) = self.data {
            Some(DatumOption::new_datum(
                PlutusData::from_bytes(hex::decode(datum).unwrap()).unwrap(),
            ))
        } else {
            None
        };
        let input: TransactionInput = TransactionInput::new(
            TransactionHash::from_hex(self.tx_hash.as_str()).unwrap(),
            self.index,
        );
        let output: TransactionOutput = TransactionOutput::new(
            Address::from_bech32(self.addr.as_str()).unwrap(),
            ExplorerValue::try_into(self.value).unwrap(),
            datum,
            None, // todo: explorer doesn't support script ref. Change to correct after explorer update
        );
        Ok(TransactionUnspentOutput::new(input, output))
    }
}
