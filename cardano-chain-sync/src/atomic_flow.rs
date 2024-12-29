use crate::cache::{LedgerCache, LinkedBlock};
use crate::client::Point;
use crate::data::{ChainUpgrade, LedgerBlockEvent};
use crate::event_source::unpack_valid_transactions_multi_era;
use async_std::prelude::Stream;
use async_std::stream::FusedStream;
use cml_chain::transaction::Transaction;
use cml_core::serialization::Deserialize;
use cml_multi_era::babbage::BabbageTransaction;
use cml_multi_era::utils::MultiEraBlockHeader;
use cml_multi_era::MultiEraBlock;
use derive_more::From;
use either::Either;
use futures::channel::{mpsc, oneshot};
use futures::{Sink, SinkExt, StreamExt};
use log::info;
use spectrum_cardano_lib::hash::hash_block_header_canonical_multi_era;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::Mutex;

type BlockEvent = LedgerBlockEvent<Vec<Either<BabbageTransaction, Transaction>>>;

pub fn atomic_block_flow<Upstream, Downstream, Cache>(
    upstream: Upstream,
    cache: Cache,
) -> (
    AtomicFlow<Upstream, Downstream, Cache>,
    impl Stream<Item = (BlockEvent, TransactionHandle)>,
) {
    let (snd, recv) = mpsc::unbounded();
    let flow = AtomicFlow::new(upstream, snd, cache);
    (flow, recv)
}

pub struct AtomicFlow<Upstream, Downstream, Cache> {
    upstream: Upstream,
    downstream: Downstream,
    cache: Cache,
}

impl<Upstream, Downstream, Cache> AtomicFlow<Upstream, Downstream, Cache> {
    pub fn new(upstream: Upstream, downstream: Downstream, cache: Cache) -> Self {
        Self {
            upstream,
            downstream,
            cache,
        }
    }

    pub async fn run(self)
    where
        Upstream: Stream<Item = ChainUpgrade<MultiEraBlock>> + Unpin + Send,
        Downstream: Sink<(BlockEvent, TransactionHandle)> + Unpin + Send,
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
                    let applied_txs = LedgerBlockEvent::RollForward(
                        unpack_valid_transactions_multi_era(blk)
                            .into_iter()
                            .map(|(tx, _, _)| tx)
                            .collect(),
                    );
                    let (snd, recv) = oneshot::channel();
                    downstream.send((applied_txs, snd.into())).await.unwrap();
                    recv.await.unwrap();
                    cache_block(&cache, &hdr, blk_bytes).await;
                }
                ChainUpgrade::RollBackward(point) => {
                    info!("Node requested rollback to point {:?}", point);
                    loop {
                        if let Some(tip) = cache.get_tip().await {
                            let rollback_finished = tip == point;
                            if !rollback_finished {
                                if let Some(LinkedBlock(block_bytes, prev_point)) =
                                    cache.get_block(tip.clone()).await
                                {
                                    let block = MultiEraBlock::from_cbor_bytes(&block_bytes)
                                        .expect("Block deserialization failed");
                                    let unapplied_txs = LedgerBlockEvent::RollBackward(
                                        unpack_valid_transactions_multi_era(block)
                                            .into_iter()
                                            .map(|(tx, _, _)| tx)
                                            .rev()
                                            .collect(),
                                    );
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
    pub fn commit(mut self) {
        let _ = self.0.send(());
    }
}

async fn cache_block<Cache: LedgerCache>(cache: &Cache, hdr: &MultiEraBlockHeader, blk_bytes: Vec<u8>) {
    let point = Point::Specific(hdr.slot(), hash_block_header_canonical_multi_era(&hdr));
    let prev_point = cache.get_tip().await.unwrap_or(Point::Origin);
    cache.set_tip(point).await;
    cache.put_block(point, LinkedBlock(blk_bytes, prev_point)).await;
}
