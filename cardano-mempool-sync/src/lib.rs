use cml_core::serialization::Deserialize;
use futures::FutureExt;
use futures::Stream;
use tokio::sync::broadcast;

use crate::client::LocalTxMonitorClient;
use crate::data::MempoolUpdate;

pub mod client;
pub mod data;

pub fn mempool_stream<'a, Tx>(
    client: LocalTxMonitorClient<Tx>,
    mut tip_reached_signal: broadcast::Receiver<bool>,
) -> impl Stream<Item = MempoolUpdate<Tx>> + Send + 'a
where
    Tx: Deserialize + Send + Sync + 'a,
{
    let wait_signal = async move {
        let _ = tip_reached_signal.recv().await;
    };
    wait_signal.map(move |_| client.stream_updates()).flatten_stream()
}
