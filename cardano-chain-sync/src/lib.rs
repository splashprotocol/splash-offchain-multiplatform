use std::time::Duration;

use async_stream::stream;
use futures::lock::Mutex;
use futures::Stream;
use futures_timer::Delay;

use crate::client::ChainSyncClient;
use crate::model::ChainUpgrade;

pub mod client;
pub mod event_source;
pub mod model;

pub fn chain_sync_stream(mut chain_sync: ChainSyncClient) -> impl Stream<Item = ChainUpgrade> {
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
            }
        }
    }
}

const THROTTLE_SECS: u64 = 1;
