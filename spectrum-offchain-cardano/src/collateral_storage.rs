use cml_chain::builders::input_builder::{InputBuilderResult, SingleInputBuilder};
use cml_chain::builders::tx_builder::TransactionUnspentOutput;

use cardano_explorer::client::Explorer;
use cardano_explorer::data::full_tx_out::ExplorerTxOut;

use crate::constants::MIN_SAFE_COLLATERAL;

pub struct CollateralStorage {
    batcher_payment_cred: String,
}

impl CollateralStorage {
    pub fn new(batcher_payment_cred: String) -> Self {
        CollateralStorage { batcher_payment_cred }
    }

    pub async fn get_collateral<'a>(self, explorer: Explorer<'a>) -> Option<InputBuilderResult> {
        let utxos = explorer.get_unspent_utxos(self.batcher_payment_cred, 0, 10).await;
        let collateral_utxo: TransactionUnspentOutput = utxos.into_iter().find_map(|utxo| {
            let utxo_value = utxo.get_value();
            if utxo_value.contains_only_ada() && utxo_value.get_ada_qty() > MIN_SAFE_COLLATERAL {
                ExplorerTxOut::try_into(utxo).ok()
            } else {
                None
            }
        })?;

        SingleInputBuilder::new(collateral_utxo.input, collateral_utxo.output)
            .payment_key()
            .ok()
    }
}
