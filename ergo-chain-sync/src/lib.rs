use std::cmp::max;
use std::sync::{Arc, Once};
use std::time::Duration;

use async_stream::stream;
use futures::lock::Mutex;
use futures::Stream;
use futures_timer::Delay;
use log::{error, info, trace};
use pin_project::pin_project;

use crate::cache::chain_cache::ChainCache;
use crate::client::node::{ErgoNetwork, Error};
use crate::model::Block;

pub mod cache;
pub mod client;
pub mod constants;
pub mod model;
pub mod rocksdb;

#[derive(Debug, Clone)]
pub enum ChainUpgrade {
    RollForward(Block),
    RollBackward(Block),
}

#[derive(Debug, Clone)]
struct SyncState {
    next_height: u32,
}

impl SyncState {
    fn upgrade(&mut self) {
        self.next_height += 1;
    }

    fn downgrade(&mut self) {
        self.next_height -= 1;
    }
}

#[async_trait::async_trait(?Send)]
pub trait InitChainSync<TChainSync> {
    async fn init(self, starting_height: u32, tip_reached_signal: Option<&'static Once>) -> TChainSync;
}

pub struct ChainSyncNonInit<'a, TClient, TCache> {
    client: &'a TClient,
    cache: TCache,
}

impl<'a, TClient, TCache> ChainSyncNonInit<'a, TClient, TCache> {
    pub fn new(client: &'a TClient, cache: TCache) -> Self {
        Self { client, cache }
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, TClient, TCache> InitChainSync<ChainSync<'a, TClient, TCache>>
    for ChainSyncNonInit<'a, TClient, TCache>
where
    TClient: ErgoNetwork,
    TCache: ChainCache,
{
    async fn init(
        self,
        starting_height: u32,
        tip_reached_signal: Option<&'a Once>,
    ) -> ChainSync<TClient, TCache> {
        ChainSync::init(starting_height, self.client, self.cache, tip_reached_signal).await
    }
}

#[pin_project]
pub struct ChainSync<'a, TClient, TCache> {
    starting_height: u32,
    client: &'a TClient,
    cache: Arc<Mutex<TCache>>,
    state: Arc<Mutex<SyncState>>,
    #[pin]
    delay: Mutex<Option<Delay>>,
    tip_reached_signal: Option<&'a Once>,
}

impl<'a, TClient, TCache> ChainSync<'a, TClient, TCache>
where
    TClient: ErgoNetwork,
    TCache: ChainCache,
{
    pub async fn init(
        starting_height: u32,
        client: &'a TClient,
        mut cache: TCache,
        tip_reached_signal: Option<&'a Once>,
    ) -> ChainSync<'a, TClient, TCache> {
        let best_block = cache.get_best_block().await;
        let start_at = if let Some(best_block) = best_block {
            trace!(target: "chain_sync", "Best block is [{}], height: {}", best_block.id, best_block.height);
            max(best_block.height, starting_height)
        } else {
            starting_height
        };
        Self {
            starting_height,
            client,
            cache: Arc::new(Mutex::new(cache)),
            state: Arc::new(Mutex::new(SyncState {
                next_height: start_at,
            })),
            delay: Mutex::new(None),
            tip_reached_signal,
        }
    }

    #[allow(clippy::await_holding_refcell_ref)]
    /// Try acquiring next upgrade from the network.
    /// `None` is returned when no upgrade is available at the moment.
    async fn try_upgrade(&self) -> Option<ChainUpgrade> {
        let next_height = { self.state.lock().await.next_height };
        trace!(target: "chain_sync", "Processing height [{}]", next_height);
        match self.client.get_block_at(next_height).await {
            Ok(api_blk) => {
                info!(
                    target: "chain_sync",
                    "Processing block [{:?}] at height [{}]",
                    api_blk.header.id,
                    next_height
                );
                info!(
                    "Processing block [{:?}] at height [{}]",
                    api_blk.header.id, next_height
                );
                let parent_id = api_blk.header.parent_id;
                let mut cache = self.cache.lock().await;
                let linked = cache.exists(parent_id).await;
                if linked || api_blk.header.height == self.starting_height {
                    trace!(target: "chain_sync", "Chain is linked, upgrading ..");
                    let blk = Block::from(api_blk);
                    cache.append_block(blk.clone()).await;
                    self.state.lock().await.upgrade();
                    return Some(ChainUpgrade::RollForward(blk));
                } else {
                    // Local chain does not link anymore
                    trace!(target: "chain_sync", "Chain does not link, downgrading ..");
                    if let Some(discarded_blk) = cache.take_best_block().await {
                        self.state.lock().await.downgrade();
                        return Some(ChainUpgrade::RollBackward(discarded_blk));
                    }
                }
            }
            Err(e) => {
                error!(target: "chain_sync", "try_upgrade: {}", e);
                if let Error::NoBlock = e {
                    // Don't want to spam console with 'no block found' messages
                } else {
                    error!("try_upgrade: {}", e);
                }
            }
        }
        None
    }
}

const THROTTLE_SECS: u64 = 1;

pub fn chain_sync_stream<'a, TClient, TCache>(
    chain_sync: ChainSync<'a, TClient, TCache>,
) -> impl Stream<Item = ChainUpgrade> + 'a
where
    TClient: ErgoNetwork + Unpin,
    TCache: ChainCache + Unpin + 'a,
{
    stream! {
            loop {
                let delay = {chain_sync.delay.lock().await.take()};
                if let Some(delay) = delay {
                    delay.await;
                }
                if let Some(upgr) = chain_sync.try_upgrade().await {
                    yield upgr;
                } else {
                    *chain_sync.delay.lock().await
                            = Some(Delay::new(Duration::from_secs(THROTTLE_SECS)));
                    if let Some(sig) = chain_sync.tip_reached_signal {
                        sig.call_once(|| {
                            trace!(target: "chain_sync", "Tip reached, waiting for new blocks ..");
                        });
                }
            }
        }
    }
}
