use cml_chain::address::Address;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::{DatumOption, TransactionInput, TransactionOutput};
use cml_core::serialization::FromBytes;
use cml_crypto::{DatumHash, TransactionHash};
use cml_crypto::CryptoError::Hex;
use serde::Deserialize;
use crate::data::value::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FullTxOut {
    tx_hash: String,
    index: u64,
    addr: String,
    value: Value,
    data: Option<String>,
    data_hash: Option<String>,
}

pub struct ParsingError;

impl TryInto<TransactionUnspentOutput> for FullTxOut {
    type Error = ParsingError;
    fn try_into(self) -> Result<TransactionUnspentOutput, Self::Error> {
        let datum =
            if let Some(hash) = self.data_hash {
                Some(DatumOption::new_hash(DatumHash::from_hex(hash.as_str()).unwrap()))
            } else if let Some(datum) = self.data {
                Some(DatumOption::new_datum(PlutusData::from_bytes(hex::decode(datum).unwrap()).unwrap()))
            } else {
                None
            };
        let input: TransactionInput = TransactionInput::new(
            TransactionHash::from_hex(self.tx_hash.as_str()).unwrap(),
            self.index,
        );
        let output: TransactionOutput = TransactionOutput::new(
            Address::from_bech32(self.addr.as_str()).unwrap(),
            Value::try_into(self.value).unwrap(),
            datum,
            None, // todo: explorer doesn't support script ref. Change to correct after explorer update
        );
        Ok(TransactionUnspentOutput::new(input, output))
    }
}