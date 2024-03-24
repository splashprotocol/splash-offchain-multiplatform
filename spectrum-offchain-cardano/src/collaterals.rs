use async_trait::async_trait;
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;

use cardano_explorer::client::Explorer;
use cardano_explorer::data::full_tx_out::ExplorerTxOut;
use spectrum_cardano_lib::collateral::Collateral;

use crate::constants::MIN_SAFE_COLLATERAL;

pub struct CollateralsViaExplorer<'a> {
    batcher_payment_cred: String,
    explorer: Explorer<'a>,
}

impl<'a> CollateralsViaExplorer<'a> {
    pub fn new(batcher_payment_cred: String, explorer: Explorer<'a>) -> CollateralsViaExplorer<'a> {
        CollateralsViaExplorer {
            batcher_payment_cred,
            explorer,
        }
    }
}

#[async_trait]
pub trait Collaterals {
    async fn get_collateral(self) -> Option<Collateral>;
}

#[async_trait]
impl<'a> Collaterals for CollateralsViaExplorer<'a> {
    async fn get_collateral(self) -> Option<Collateral> {
        let utxos = self
            .explorer
            .get_unspent_utxos(self.batcher_payment_cred, 0, 300) //todo: check limit
            .await;
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
            .map(Collateral)
    }
}

#[cfg(test)]
pub mod tests {
    use async_trait::async_trait;
    use cml_chain::builders::input_builder::{SingleInputBuilder};
    use cml_chain::transaction::{TransactionInput, TransactionOutput};
    use cml_crypto::TransactionHash;
    use spectrum_cardano_lib::collateral::Collateral;

    use crate::collaterals::Collaterals;

    fn genesis_id() -> TransactionHash {
        TransactionHash::from([0u8; TransactionHash::BYTE_COUNT])
    }

    pub struct MockBasedRequestor {
        output: TransactionOutput,
    }

    impl MockBasedRequestor {
        pub fn new(output: TransactionOutput) -> MockBasedRequestor {
            MockBasedRequestor { output }
        }
    }

    #[async_trait]
    impl Collaterals for MockBasedRequestor {
        async fn get_collateral(self) -> Option<Collateral> {
            let input = TransactionInput::new(genesis_id(), 0);

            SingleInputBuilder::new(input, self.output)
                .payment_key()
                .ok()
                .map(Collateral)
        }
    }
}
