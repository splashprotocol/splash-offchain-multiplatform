use std::collections::{HashMap, HashSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_stream::stream;
use ergo_lib::chain::transaction::{Transaction, TxId};
use futures::stream::select_all;
use futures::{Stream, StreamExt};
use log::info;
use tokio::sync::Mutex;
use wasm_timer::Delay;

use ergo_chain_sync::cache::chain_cache::ChainCache;
use ergo_chain_sync::client::node::ErgoNetwork;
use ergo_chain_sync::model::Block;
use ergo_chain_sync::{chain_sync_stream, ChainSync, ChainUpgrade, InitChainSync};

#[derive(Debug, Clone)]
pub enum MempoolUpdate {
    /// Tx was accepted to mempool.
    TxAccepted(Transaction),
    /// Tx was discarded.
    TxWithdrawn(Transaction),
    /// Tx was confirmed.
    TxConfirmed(Transaction),
}

#[derive(Debug, Clone)]
struct SyncState {
    latest_blocks: VecDeque<HashSet<TxId>>,
    mempool_projection: HashMap<TxId, Transaction>,
    pending_updates: VecDeque<MempoolUpdate>,
}

impl SyncState {
    fn empty() -> Self {
        Self {
            latest_blocks: VecDeque::new(),
            mempool_projection: HashMap::new(),
            pending_updates: VecDeque::new(),
        }
    }
}

const KEEP_LAST_BLOCKS: usize = 10;

impl SyncState {
    fn push_block(&mut self, blk: Block) {
        self.latest_blocks
            .push_back(HashSet::from_iter(blk.transactions.into_iter().map(|tx| tx.id())));
        if self.latest_blocks.len() > KEEP_LAST_BLOCKS {
            self.latest_blocks.pop_front();
        }
    }

    fn pop_block(&mut self) {
        self.latest_blocks.pop_back();
    }
}

#[derive(Debug, Copy, Clone)]
pub struct MempoolSyncConf {
    pub sync_interval: Duration,
}

const TXS_PER_REQUEST: usize = 100;

#[allow(clippy::await_holding_refcell_ref)]
async fn sync<TClient: ErgoNetwork>(client: &TClient, state: Arc<Mutex<SyncState>>) {
    let mut pool: Vec<Transaction> = Vec::new();
    let mut offset = 0;
    loop {
        let txs = client.fetch_mempool(offset, TXS_PER_REQUEST).await;
        match txs {
            Ok(mut mempool_txs) => {
                let num_txs = mempool_txs.len();
                pool.append(mempool_txs.as_mut());
                if num_txs < TXS_PER_REQUEST {
                    break;
                }
                offset += num_txs
            }
            Err(error) => {
                info!(
                    target: "mempool_sync",
                    "# Failed to request next mempool transactions: {}",
                    error,
                );
            }
        }
    }
    let new_pool_ids = pool.iter().map(|tx| tx.id()).collect::<HashSet<_>>();
    let mut state = state.lock().await;
    let old_pool_ids = state.mempool_projection.keys().cloned().collect::<HashSet<_>>();
    let elim_txs = old_pool_ids.difference(&new_pool_ids);
    'check_withdrawn: for tx_id in elim_txs {
        if let Some(tx) = state.mempool_projection.remove(tx_id) {
            for blk in state.latest_blocks.iter() {
                if blk.contains(tx_id) {
                    state.pending_updates.push_back(MempoolUpdate::TxConfirmed(tx));
                    continue 'check_withdrawn;
                }
            }
            state.pending_updates.push_back(MempoolUpdate::TxWithdrawn(tx));
        }
    }
    for tx in pool {
        if state.mempool_projection.contains_key(&tx.id()) {
            continue;
        }
        state.mempool_projection.insert(tx.id(), tx.clone());
        state.pending_updates.push_back(MempoolUpdate::TxAccepted(tx));
    }
}

const START_HEIGHT: usize = 0;

pub async fn mempool_sync_stream<'a, TChainSyncMaker, TClient, TCache>(
    conf: MempoolSyncConf,
    chain_sync_maker: TChainSyncMaker,
    client: &'a TClient,
) -> impl Stream<Item = MempoolUpdate> + 'a
where
    TClient: ErgoNetwork + Unpin + 'a,
    TCache: ChainCache + Unpin + 'a,
    TChainSyncMaker: InitChainSync<ChainSync<'a, TClient, TCache>>,
{
    let chain_tip_height = client.get_best_height().await;
    let start_at = match chain_tip_height {
        Ok(height) => height as usize - KEEP_LAST_BLOCKS,
        Err(error) => {
            info!(
                target: "mempool_sync",
                "# Failed to request best height: {}",
                error,
            );
            START_HEIGHT
        }
    };
    let chain_sync = chain_sync_maker.init(start_at as u32, None).await;
    let state = Arc::new(Mutex::new(SyncState::empty()));
    let joined_stream = select_all(vec![
        boxed(sync_ledger(chain_sync_stream(chain_sync), Arc::clone(&state)).map(move |_| None)),
        boxed(sync_mempool(conf, client, state)),
    ]);
    joined_stream.filter_map(futures::future::ready)
}

fn sync_ledger<'a, S>(upstream: S, state: Arc<Mutex<SyncState>>) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = ChainUpgrade> + 'a,
{
    upstream.then(move |upgrade| {
        let state = Arc::clone(&state);
        async move {
            let mut state = state.lock().await;
            match upgrade {
                ChainUpgrade::RollForward(blk) => {
                    state.push_block(blk);
                }
                ChainUpgrade::RollBackward(_) => {
                    state.pop_block();
                }
            }
        }
    })
}

#[allow(clippy::needless_lifetimes)] // Note: clippy false-positive
fn sync_mempool<'a, TClient>(
    conf: MempoolSyncConf,
    client: &'a TClient,
    state: Arc<Mutex<SyncState>>,
) -> impl Stream<Item = Option<MempoolUpdate>> + 'a
where
    TClient: ErgoNetwork,
{
    stream! {
        loop {
            let mut st = state.lock().await;
            let maybe_upd = st.pending_updates.pop_front();
            drop(st);
            if let Some(upd) = maybe_upd { // First, try to pop updates
                yield Some(upd)
            } else { // Wait otherwise
                let _ = Delay::new(conf.sync_interval).await;
            }
            sync(client, Arc::clone(&state)).await;
        }
    }
}

fn boxed<'a, T>(s: impl Stream<Item = T> + 'a) -> Pin<Box<dyn Stream<Item = T> + 'a>> {
    Box::pin(s)
}
