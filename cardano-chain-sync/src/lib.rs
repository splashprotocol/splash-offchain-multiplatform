use std::sync::Once;
use std::time::Duration;

use async_stream::stream;
use futures::lock::Mutex;
use futures::Stream;
use futures_timer::Delay;
use log::trace;

use crate::client::ChainSyncClient;
use crate::data::ChainUpgrade;

pub mod client;
pub mod data;
pub mod event_source;

pub fn chain_sync_stream<'a>(
    mut chain_sync: ChainSyncClient,
    tip_reached_signal: Option<&'a Once>,
) -> impl Stream<Item = ChainUpgrade> + 'a {
    let delay_mux: Mutex<Option<Delay>> = Mutex::new(None);
    stream! {
        loop {
            let delay = {delay_mux.lock().await.take()};
            if let Some(delay) = delay {
                delay.await;
            }
            if let Some(upgr) = chain_sync.try_pull_next().await {
                yield upgr;
            } else {
                *delay_mux.lock().await = Some(Delay::new(Duration::from_secs(THROTTLE_SECS)));
                if let Some(sig) = tip_reached_signal {
                    sig.call_once(|| {
                        trace!(target: "chain_sync", "Tip reached, waiting for new blocks ..");
                    });
                }
            }
        }
    }
}

const THROTTLE_SECS: u64 = 1;
