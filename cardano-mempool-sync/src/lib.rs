use crate::client::LocalTxMonitorClient;
use crate::data::MempoolUpdate;
use async_primitives::beacon::Beacon;
use cml_core::serialization::Deserialize;
use futures::stream::select;
use futures::Stream;
use futures::{FutureExt, StreamExt};
use spectrum_offchain::tx_hash::CanonicalHash;
use spectrum_offchain_cardano::tx_tracker::TxTracker;

pub mod client;
pub mod data;

pub fn mempool_stream<'a, Tx, Tracker, StFailedTxs>(
    client: LocalTxMonitorClient<Tx>,
    tx_tracker: Tracker,
    failed_txs: StFailedTxs,
    state_synced: Beacon,
) -> impl Stream<Item = MempoolUpdate<Tx>> + Send + 'a
where
    Tx: CanonicalHash + Clone + Deserialize + Send + Sync + 'a,
    Tracker: TxTracker<Tx::Hash, Tx> + Clone + Send + Sync + 'a,
    StFailedTxs: Stream<Item = Tx> + Send + 'a,
{
    let accepted_txs = client
        .stream_updates()
        .then(move |tx| {
            let mut tracker = tx_tracker.clone();
            async move {
                tracker.track(tx.canonical_hash(), tx.clone()).await;
                tx
            }
        })
        .map(MempoolUpdate::TxAccepted);
    let failed_txs = failed_txs.map(MempoolUpdate::TxDropped);
    state_synced
        .once(true)
        .map(move |_| select(accepted_txs, failed_txs))
        .flatten_stream()
}
