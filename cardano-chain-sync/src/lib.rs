use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::client::ChainSyncClient;
use crate::data::ChainUpgrade;
use async_primitives::beacon::Beacon;
use async_stream::stream;
use cml_core::serialization::Deserialize;
use futures::lock::Mutex;
use futures::Stream;
use futures_timer::Delay;
use log::trace;
use tokio::sync::broadcast;

pub mod atomic_flow;
pub mod cache;
pub mod client;
pub mod data;
pub mod event_source;

pub fn chain_sync_stream<'a, Block>(
    mut chain_sync: ChainSyncClient<Block>,
    state_synced: Beacon,
) -> impl Stream<Item = ChainUpgrade<Block>> + 'a
where
    Block: Deserialize + 'a,
{
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
                trace!(target: "chain_sync", "Tip reached, waiting for new blocks ..");
                *delay_mux.lock().await = Some(Delay::new(Duration::from_secs(THROTTLE_SECS)));
                state_synced.alter(true);
            }
        }
    }
}

const THROTTLE_SECS: u64 = 1;
