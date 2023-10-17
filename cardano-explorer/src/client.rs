use crate::data::ExplorerConfig;
use cml_chain::transaction::TransactionInput;
use spectrum_cardano_lib::OutputRef;
use tracing_subscriber::fmt::format::Full;

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
        let tx_id = TransactionInput::from(output_ref).transaction_id.to_hex();

        let out_id = TransactionInput::from(output_ref).index;

        let request_url = self.explorer_config.url.to_owned()
            + "/cardano/mainnet/v1/outputs/"
            + tx_id.as_str()
            + ":"
            + out_id.to_string().as_str();

        let raw_response = reqwest::get(request_url).await.ok()?;

        raw_response.json::<FullTxOut>().await.ok()
    }

    // explorer don't support extracting unspent utxos by address, only address payment cred
    pub async fn get_unspent_utxos(self, payment_cred: String, min_index: u32, limit: u32) -> Vec<FullTxOut> {
        let request_url = self.explorer_config.url.to_owned()
            + "/cardano/mainnet/v1/outputs/unspent/byPaymentCred/"
            + payment_cred.as_str()
            + "/?offset="
            + min_index.to_string().as_str()
            + "&limit="
            + limit.to_string().as_str();

        reqwest::get(request_url)
            .await
            .ok()
            .unwrap()
            .json::<Items<FullTxOut>>()
            .await
            .ok()
            .map_or(Vec::new(), |items| items.get_items())
    }
}
