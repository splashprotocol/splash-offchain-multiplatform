use crate::data::ExplorerConfig;
use cml_chain::transaction::TransactionInput;
use serde::de::DeserializeOwned;
use spectrum_cardano_lib::OutputRef;

use crate::data::full_tx_out::FullTxOut;
use crate::data::items::Items;

#[derive(Copy, Clone)]
pub struct Explorer<'a> {
    explorer_config: ExplorerConfig<'a>,
}

impl<'a> Explorer<'a> {
    pub fn new(config: ExplorerConfig<'a>) -> Self {
        Explorer {
            explorer_config: config,
        }
    }

    pub async fn get_utxo(self, output_ref: OutputRef) -> Option<FullTxOut> {
        let request_url = format!(
            "{}/cardano/mainnet/v1/outputs/{}:{}",
            self.explorer_config.url.to_owned(),
            TransactionInput::from(output_ref).transaction_id.to_hex(),
            TransactionInput::from(output_ref).index
        );

        Self::get_request::<FullTxOut>(request_url).await
    }

    // explorer doesn't support extracting unspent utxos by address, only address payment cred
    pub async fn get_unspent_utxos(self, payment_cred: String, min_index: u32, limit: u32) -> Vec<FullTxOut> {
        let request_url = format!(
            "{}/cardano/mainnet/v1/outputs/unspent/byPaymentCred/{}/?offset={}&limit={}",
            self.explorer_config.url.to_owned(),
            payment_cred.as_str(),
            min_index.to_string().as_str(),
            limit.to_string().as_str()
        );

        Self::get_request::<Items<FullTxOut>>(request_url)
            .await
            .map_or(Vec::new(), |items| items.get_items())
    }

    async fn get_request<T: DeserializeOwned>(url: String) -> Option<T> {
        reqwest::get(url).await.ok().unwrap().json::<T>().await.ok()
    }
}
