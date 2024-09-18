use async_trait::async_trait;
use bloom_offchain_cardano::event_sink::processed_tx::TxViewAtEraBoundary;
use cardano_chain_sync::data::LedgerTxEvent;
use cml_crypto::TransactionHash;
use cml_multi_era::babbage::BabbageTransaction;
use spectrum_offchain::event_sink::event_handler::EventHandler;

/// This event handler simply forwards the [`LedgerTxEvent`] to the `Behaviour` since the
/// deserialization of [`crate::entities::onchain::weighting_poll::WeightingPoll`]
/// requires the current epoch, which is only obtainable from `Behaviour`
pub struct DaoHandler {
    tx: tokio::sync::mpsc::Sender<LedgerTxEvent<TxViewAtEraBoundary>>,
}

impl DaoHandler {
    pub fn new(tx: tokio::sync::mpsc::Sender<LedgerTxEvent<TxViewAtEraBoundary>>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl EventHandler<LedgerTxEvent<TxViewAtEraBoundary>> for DaoHandler {
    async fn try_handle(
        &mut self,
        ev: LedgerTxEvent<TxViewAtEraBoundary>,
    ) -> Option<LedgerTxEvent<TxViewAtEraBoundary>> {
        self.tx.send(ev).await.unwrap();
        None
    }
}
