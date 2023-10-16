use cml_chain::address::{EnterpriseAddress};
use cml_chain::certs::StakeCredential;
use cml_chain::transaction::{ConwayFormatTxOut, TransactionOutput};
use cml_chain::{Coin, Value};
use cml_chain::assets::MultiAsset;
use cml_chain::genesis::network_info::NetworkInfo;
use cml_crypto::Ed25519KeyHash;

use spectrum_offchain::ledger::IntoLedger;

#[derive(Debug, Clone)]
pub struct BatcherProfit {
    pub ada_profit: Coin,
    pub batcher_pkh: Ed25519KeyHash
}

impl BatcherProfit {
    pub fn of(coin: Coin, batcher_pkh: Ed25519KeyHash) -> Self {
        Self { ada_profit: coin, batcher_pkh }
    }
}

impl IntoLedger<TransactionOutput, ()> for BatcherProfit {
    fn into_ledger(self, _ctx: ()) -> TransactionOutput {

        let addr = EnterpriseAddress::new(
            NetworkInfo::mainnet().network_id(),
            StakeCredential::new_pub_key(self.batcher_pkh),
        ).to_address();

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: addr,
            amount: Value::new(self.ada_profit, MultiAsset::new()),
            datum_option: None,
            script_reference: None,
            encodings: None,
        })
    }
}
