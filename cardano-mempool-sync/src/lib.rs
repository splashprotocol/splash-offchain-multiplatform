use crate::client::LocalTxMonitorClient;
use crate::data::MempoolUpdate;
use cml_core::serialization::Deserialize;
use futures::stream::select;
use futures::Stream;
use futures::{FutureExt, StreamExt};
use spectrum_offchain::tx_hash::CanonicalHash;
use spectrum_offchain::tx_tracker::TxTracker;
use tokio::sync::broadcast;

pub mod client;
pub mod data;

pub fn mempool_stream<'a, Tx, Tracker, StFailedTxs>(
    client: LocalTxMonitorClient<Tx>,
    tx_tracker: Tracker,
    failed_txs: StFailedTxs,
    mut tip_reached_signal: broadcast::Receiver<bool>,
) -> impl Stream<Item = MempoolUpdate<Tx>> + Send + 'a
where
    Tx: CanonicalHash + Clone + Deserialize + Send + Sync + 'a,
    Tracker: TxTracker<Tx::Hash, Tx> + Clone + Send + Sync + 'a,
    StFailedTxs: Stream<Item = Tx> + Send + 'a,
{
    let wait_signal = async move {
        let _ = tip_reached_signal.recv().await;
    };
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
    wait_signal
        .map(move |_| select(accepted_txs, failed_txs))
        .flatten_stream()
}
