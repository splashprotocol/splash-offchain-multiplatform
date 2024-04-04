use cml_chain::builders::input_builder::{InputBuilderResult, SingleInputBuilder};
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use derive_more::{From, Into};

#[derive(Clone, Debug, Into, From)]
pub struct Collateral(TransactionUnspentOutput);

impl From<Collateral> for InputBuilderResult {
    fn from(Collateral(utxo): Collateral) -> Self {
        SingleInputBuilder::new(utxo.input, utxo.output)
            .payment_key()
            .expect("UTxO must be P2PK")
    }
}
