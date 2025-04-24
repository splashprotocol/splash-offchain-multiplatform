use crate::cache::{LedgerCache, LinkedBlock};
use crate::client::Point;
use crate::data::ChainUpgrade;
use crate::event_source::unpack_valid_transactions_multi_era;
use async_std::prelude::Stream;
use cml_chain::transaction::Transaction;
use cml_core::serialization::Deserialize;
use cml_core::Slot;
use cml_multi_era::babbage::BabbageTransaction;
use cml_multi_era::utils::MultiEraBlockHeader;
use cml_multi_era::MultiEraBlock;
use derive_more::From;
use either::Either;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::channel::{mpsc, oneshot};
use futures::{Sink, SinkExt, StreamExt};
use log::{info, trace};
use spectrum_cardano_lib::hash::hash_block_header_canonical_multi_era;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub enum BlockEvents<T> {
    RollForward {
        events: Vec<T>,
        block_num: u64,
        block_slot: Slot,
    },
    RollBackward {
        events: Vec<T>,
        block_num: u64,
        block_slot: Slot,
    },
}

impl<T> BlockEvents<T> {
    pub fn map<T2, F>(self, f: F) -> BlockEvents<T2>
    where
        F: FnOnce(Vec<T>) -> Vec<T2>,
    {
        match self {
            BlockEvents::RollForward {
                events,
                block_num,
                block_slot,
            } => BlockEvents::RollForward {
                events: f(events),
                block_num,
                block_slot,
            },
            BlockEvents::RollBackward {
                events,
                block_num,
                block_slot,
            } => BlockEvents::RollBackward {
                events: f(events),
                block_num,
                block_slot,
            },
        }
    }

    pub fn block_slot(self) -> Slot {
        match self {
            BlockEvents::RollForward { block_slot, .. } => block_slot,
            BlockEvents::RollBackward { block_slot, .. } => block_slot,
        }
    }
}

pub fn atomic_block_flow<Upstream, Cache>(
    upstream: Upstream,
    cache: Arc<Mutex<Cache>>,
) -> (
    AtomicFlow<
        Upstream,
        UnboundedSender<(
            BlockEvents<Either<BabbageTransaction, Transaction>>,
            TransactionHandle,
        )>,
        Cache,
    >,
    UnboundedReceiver<(
        BlockEvents<Either<BabbageTransaction, Transaction>>,
        TransactionHandle,
    )>,
) {
    let (snd, recv) = mpsc::unbounded();
    let flow = AtomicFlow::new(upstream, snd, cache);
    (flow, recv)
}

pub struct AtomicFlow<Upstream, Downstream, Cache> {
    upstream: Upstream,
    downstream: Downstream,
    cache: Arc<Mutex<Cache>>,
}

impl<Upstream, Downstream, Cache> AtomicFlow<Upstream, Downstream, Cache> {
    pub fn new(upstream: Upstream, downstream: Downstream, cache: Arc<Mutex<Cache>>) -> Self {
        Self {
            upstream,
            downstream,
            cache,
        }
    }

    pub async fn run(self)
    where
        Upstream: Stream<Item = ChainUpgrade<MultiEraBlock>> + Unpin + Send,
        Downstream: Sink<(
                BlockEvents<Either<BabbageTransaction, Transaction>>,
                TransactionHandle,
            )> + Unpin
            + Send,
        Downstream::Error: Debug,
        Cache: LedgerCache + Send,
    {
        let Self {
            upstream,
            mut downstream,
            cache,
        } = self;
        let mut upstream = upstream.fuse();
        loop {
            let upgrade = upstream.select_next_some().await;
            match upgrade {
                ChainUpgrade::RollForward { blk, blk_bytes, .. } => {
                    let hdr = blk.header();
                    info!(
                        "Scanning Block {}",
                        hash_block_header_canonical_multi_era(&hdr).to_hex()
                    );
                    let applied_txs = BlockEvents::RollForward {
                        events: unpack_valid_transactions_multi_era(blk)
                            .into_iter()
                            .map(|(tx, _, _, _)| tx)
                            .collect(),
                        block_num: hdr.block_number(),
                        block_slot: hdr.slot(),
                    };
                    let (snd, recv) = oneshot::channel();
                    downstream.send((applied_txs, snd.into())).await.unwrap();
                    trace!("Transaction started");
                    recv.await.unwrap();
                    trace!("Transaction completed");
                    cache_block(cache.clone(), &hdr, blk_bytes).await;
                }
                ChainUpgrade::RollBackward(point) => {
                    info!("Node requested rollback to point {:?}", point);
                    loop {
                        let cache = cache.lock().await;
                        if let Some(tip) = cache.get_tip().await {
                            let rollback_finished = tip == point;
                            if !rollback_finished {
                                if let Some(LinkedBlock(block_bytes, prev_point)) =
                                    cache.get_block(tip.clone()).await
                                {
                                    let block = MultiEraBlock::from_cbor_bytes(&block_bytes)
                                        .expect("Block deserialization failed");
                                    let block_num = block.header().block_number();
                                    let block_slot = block.header().slot();
                                    let unapplied_txs = BlockEvents::RollBackward {
                                        events: unpack_valid_transactions_multi_era(block)
                                            .into_iter()
                                            .map(|(tx, _, _, _)| tx)
                                            .rev()
                                            .collect(),
                                        block_num,
                                        block_slot,
                                    };
                                    let (snd, recv) = oneshot::channel();
                                    downstream.send((unapplied_txs, snd.into())).await.unwrap();
                                    recv.await.unwrap();
                                    cache.delete(tip).await;
                                    cache.set_tip(prev_point).await;
                                    continue;
                                }
                            }
                        }
                        info!("Rolled back to point {:?}", point);
                        break;
                    }
                }
            }
        }
    }
}

/// A handle allowing to signal that the transaction is completed.
#[derive(From)]
pub struct TransactionHandle(oneshot::Sender<()>);
impl TransactionHandle {
    pub fn commit(self) {
        let _ = self.0.send(());
    }
}

async fn cache_block<Cache: LedgerCache>(
    cache: Arc<Mutex<Cache>>,
    hdr: &MultiEraBlockHeader,
    blk_bytes: Vec<u8>,
) {
    let point = Point::Specific(hdr.slot(), hash_block_header_canonical_multi_era(&hdr));
    let cache = cache.lock().await;
    let prev_point = cache.get_tip().await.unwrap_or(Point::Origin);
    cache.set_tip(point).await;
    cache.put_block(point, LinkedBlock(blk_bytes, prev_point)).await;
}
