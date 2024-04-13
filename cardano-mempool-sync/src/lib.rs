use cml_core::serialization::Deserialize;
use futures::Stream;
use futures::FutureExt;
use tokio::sync::broadcast;

use crate::client::LocalTxMonitorClient;
use crate::data::MempoolUpdate;

pub mod client;
pub mod data;

pub fn mempool_stream<'a, Tx>(
    client: &'a LocalTxMonitorClient<Tx>,
    mut tip_reached_signal: broadcast::Receiver<bool>,
) -> impl Stream<Item=MempoolUpdate<Tx>> + 'a
    where
        Tx: Deserialize + 'a,
{
    let wait_signal = async move { let _ = tip_reached_signal.recv().await; };
    wait_signal.map(move |_| client.stream_updates()).flatten_stream()
}
