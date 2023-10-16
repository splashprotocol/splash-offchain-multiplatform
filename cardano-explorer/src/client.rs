use cml_chain::transaction::TransactionInput;
use tracing_subscriber::fmt::format::Full;
use spectrum_cardano_lib::OutputRef;
use crate::data::ExplorerConfig;

use crate::data::full_tx_out::FullTxOut;

#[derive(Copy, Clone)]
pub struct Explorer<'a> {
    explorer_config: ExplorerConfig<'a>,
}

impl<'a> Explorer<'a> {
    pub fn new(config: ExplorerConfig<'a>) -> Self {
        Explorer {
            explorer_config: config
        }
    }

    pub async fn get_utxo(self, output_ref: OutputRef) -> Option<FullTxOut> {
        let txId =
            TransactionInput::from(output_ref).transaction_id.to_hex();

        let outId =
            TransactionInput::from(output_ref).index;

        let request_url = self.explorer_config.url.to_owned() + "/cardano/mainnet/v1/outputs/" + txId.as_str() + ":" + outId.to_string().as_str();

        let raw_response = reqwest::get(request_url)
            .await.ok()?;

        raw_response.json::<FullTxOut>().await.ok()
    }
}