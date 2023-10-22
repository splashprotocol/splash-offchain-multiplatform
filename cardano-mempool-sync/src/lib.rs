use std::sync::Once;
use std::time::Duration;

use async_stream::stream;
use cml_chain::transaction::Transaction;
use futures::lock::Mutex;
use futures::Stream;
use futures_timer::Delay;

use crate::client::LocalTxMonitorClient;
use crate::data::MempoolUpdate;

pub mod client;
pub mod data;

pub fn mempool_stream<'a>(
    mut client: LocalTxMonitorClient,
    tip_reached_signal: Option<&'a Once>,
) -> impl Stream<Item = MempoolUpdate<Transaction>> + 'a {
    let delay_mux: Mutex<Option<Delay>> = Mutex::new(None);
    stream! {
        loop {
            let delay = {delay_mux.lock().await.take()};
            if let Some(delay) = delay {
                delay.await;
            }
            let is_active = if let Some(sig) = tip_reached_signal {
                sig.is_completed()
            } else {
                true
            };
            if is_active {
                if let Some(upgr) = client.try_pull_next().await {
                    yield upgr;
                } else {
                    *delay_mux.lock().await = Some(Delay::new(Duration::from_secs(THROTTLE_AWAIT_MILLIS)));
                }
            } else {
                *delay_mux.lock().await = Some(Delay::new(Duration::from_secs(THROTTLE_IDLE_MILLIS)));
            }
        }
    }
}

const THROTTLE_AWAIT_MILLIS: u64 = 100;
const THROTTLE_IDLE_MILLIS: u64 = 1000;
