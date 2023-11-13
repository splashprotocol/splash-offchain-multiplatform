use std::sync::Once;
use std::time::Duration;

use async_std::stream;
use cml_core::serialization::Deserialize;
use futures::{Stream, StreamExt};
use futures_timer::Delay;

use crate::client::LocalTxMonitorClient;
use crate::data::MempoolUpdate;

pub mod client;
pub mod data;

pub fn mempool_stream<'a, Tx>(
    client: &'a LocalTxMonitorClient<Tx>,
    tip_reached_signal: Option<&'a Once>,
) -> impl Stream<Item = MempoolUpdate<Tx>> + 'a
where
    Tx: Deserialize + 'a,
{
    let wait_chain_sync = async move {
        let mut delay_mux: Option<Delay> = None;
        loop {
            if let Some(delay) = delay_mux.take() {
                delay.await;
            }
            let done = if let Some(sig) = tip_reached_signal {
                sig.is_completed()
            } else {
                true
            };
            if done {
                break;
            } else {
                delay_mux = Some(Delay::new(Duration::from_millis(THROTTLE_IDLE_MILLIS)));
            }
        }
    };
    stream::once(wait_chain_sync).flat_map(move |_| client.stream_updates())
}

const THROTTLE_IDLE_MILLIS: u64 = 1000;
