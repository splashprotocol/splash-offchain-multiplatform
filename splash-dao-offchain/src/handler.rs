use async_trait::async_trait;
use bloom_offchain_cardano::event_sink::tx_view::TxViewMut;
use cardano_chain_sync::data::LedgerTxEvent;
use spectrum_offchain::event_sink::event_handler::EventHandler;

/// This event handler simply forwards the [`LedgerTxEvent`] to the `Behaviour` since the
/// deserialization of [`crate::entities::onchain::weighting_poll::WeightingPoll`]
/// requires the current epoch, which is only obtainable from `Behaviour`
pub struct DaoHandler {
    tx: tokio::sync::mpsc::Sender<LedgerTxEvent<TxViewMut>>,
}

impl DaoHandler {
    pub fn new(tx: tokio::sync::mpsc::Sender<LedgerTxEvent<TxViewMut>>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl EventHandler<LedgerTxEvent<TxViewMut>> for DaoHandler {
    async fn try_handle(&mut self, ev: LedgerTxEvent<TxViewMut>) -> Option<LedgerTxEvent<TxViewMut>> {
        self.tx.send(ev.clone()).await.unwrap();
        Some(ev)
    }
}
