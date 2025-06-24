use crate::index::UtxoIndex;
use async_trait::async_trait;
use bloom_offchain_cardano::event_sink::tx_view::TxViewMut;
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_mempool_sync::data::MempoolUpdate;
use cml_chain::Slot;
use spectrum_cardano_lib::address::AddressExtension;
use spectrum_offchain::event_sink::event_handler::EventHandler;

#[derive(Clone)]
pub struct TxHandler<Index> {
    index: Index,
}

impl<Index> TxHandler<Index> {
    pub fn new(index: Index) -> Self {
        Self { index }
    }
}

#[async_trait]
impl<Index> EventHandler<LedgerTxEvent<TxViewMut>> for TxHandler<Index>
where
    Index: UtxoIndex + Send + Sync,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent<TxViewMut>) -> Option<LedgerTxEvent<TxViewMut>> {
        match ev {
            LedgerTxEvent::TxApplied { tx, slot, .. } => {
                apply_tx(&self.index, tx, Some(slot)).await;
            }
            LedgerTxEvent::TxUnapplied { tx, .. } => unapply_tx(&self.index, tx).await,
        }
        None
    }
}

#[async_trait]
impl<Index> EventHandler<MempoolUpdate<TxViewMut>> for TxHandler<Index>
where
    Index: UtxoIndex + Send + Sync,
{
    async fn try_handle(&mut self, ev: MempoolUpdate<TxViewMut>) -> Option<MempoolUpdate<TxViewMut>> {
        match ev {
            MempoolUpdate::TxAccepted(tx) => apply_tx(&self.index, tx, None).await,
            MempoolUpdate::TxDropped(tx) => unapply_tx(&self.index, tx).await,
        }
        None
    }
}

async fn apply_tx<Index>(
    index: &Index,
    TxViewMut {
        hash,
        inputs,
        outputs,
        ..
    }: TxViewMut,
    settled_at: Option<Slot>,
) where
    Index: UtxoIndex + Send,
{
    index
        .apply(
            hash,
            inputs.into_iter().map(|i| i.into()).collect(),
            outputs
                .into_iter()
                .filter(|(_, o)| o.address().script_hash().is_none())
                .collect(),
            settled_at,
        )
        .await;
}

async fn unapply_tx<Index>(
    index: &Index,
    TxViewMut {
        hash,
        inputs,
        outputs,
        ..
    }: TxViewMut,
) where
    Index: UtxoIndex + Send,
{
    index
        .unapply(
            hash,
            inputs.into_iter().map(|i| i.into()).collect(),
            outputs
                .into_iter()
                .filter(|(_, o)| o.address().script_hash().is_none())
                .collect(),
        )
        .await;
}
