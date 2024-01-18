use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;

use async_stream::stream;
use cml_core::Slot;
use cml_multi_era::babbage::{BabbageBlock, BabbageTransaction};
use futures::stream::StreamExt;
use futures::{stream, Stream};
use log::warn;
use tokio::sync::Mutex;

use spectrum_cardano_lib::hash::hash_block_header_canonical;

use crate::cache::{LedgerCache, Linked};
use crate::client::Point;
use crate::data::{ChainUpgrade, LedgerBlockEvent, LedgerTxEvent};

/// Stream ledger updates as individual transactions.
pub fn ledger_transactions<'a, S, Cache>(
    cache: Arc<Mutex<Cache>>,
    upstream: S,
    // Rollbacks will not be handled until the specified slot is reached.
    handle_rollbacks_after: Slot,
) -> impl Stream<Item = LedgerTxEvent<BabbageTransaction>> + 'a
where
    S: Stream<Item = ChainUpgrade<BabbageBlock>> + 'a,
    Cache: LedgerCache<BabbageBlock> + 'a,
{
    upstream
        .then(move |u| process_upstream_by_txs(Arc::clone(&cache), u, handle_rollbacks_after))
        .flatten()
}

/// Stream ledger updates as blocks.
pub fn ledger_blocks<'a, S, Cache>(
    cache: Arc<Mutex<Cache>>,
    upstream: S,
    // Rollbacks will not be handled until the specified slot is reached.
    handle_rollbacks_after: Slot,
) -> impl Stream<Item = LedgerBlockEvent<BabbageBlock>> + 'a
where
    S: Stream<Item = ChainUpgrade<BabbageBlock>> + 'a,
    Cache: LedgerCache<BabbageBlock> + 'a,
{
    upstream.flat_map(move |u| process_upstream_by_blocks(Arc::clone(&cache), u, handle_rollbacks_after))
}

async fn process_upstream_by_txs<'a, Cache>(
    cache: Arc<Mutex<Cache>>,
    upgr: ChainUpgrade<BabbageBlock>,
    handle_rollbacks_after: Slot,
) -> Pin<Box<dyn Stream<Item = LedgerTxEvent<BabbageTransaction>> + 'a>>
where
    Cache: LedgerCache<BabbageBlock> + 'a,
{
    match upgr {
        ChainUpgrade::RollForward(blk) => {
            if blk.header.header_body.slot > handle_rollbacks_after {
                cache_block(cache, blk.clone()).await;
            } else {
                cache_point(cache, &blk).await;
            }
            let applied_txs: Vec<_> = unpack_valid_transactions(blk)
                .map(|(tx, slot)| LedgerTxEvent::TxApplied { tx, slot })
                .collect();
            Box::pin(stream::iter(applied_txs))
        }
        ChainUpgrade::RollBackward(point) if point.get_slot() > handle_rollbacks_after => {
            Box::pin(rollback(cache, point.into()).flat_map(|blk| {
                let unapplied_txs: Vec<_> = unpack_valid_transactions(blk)
                    .map(|(tx, _)| LedgerTxEvent::TxUnapplied(tx))
                    .rev()
                    .collect();
                stream::iter(unapplied_txs)
            }))
        }
        ChainUpgrade::RollBackward(_) => {
            warn!("Node requested rollback while rollbacks are disabled.");
            Box::pin(stream::empty())
        }
    }
}

async fn cache_block<Cache: LedgerCache<BabbageBlock>>(cache: Arc<Mutex<Cache>>, blk: BabbageBlock) {
    let cache = cache.lock().await;
    let point = Point::Specific(
        blk.header.header_body.slot,
        hash_block_header_canonical(&blk.header),
    );
    let prev_point = cache.get_tip().await.unwrap_or(Point::Origin);
    cache.set_tip(point).await;
    cache.put_block(point, Linked(blk, prev_point)).await;
}

async fn cache_point<Cache: LedgerCache<BabbageBlock>>(cache: Arc<Mutex<Cache>>, blk: &BabbageBlock) {
    let cache = cache.lock().await;
    let point = Point::Specific(
        blk.header.header_body.slot,
        hash_block_header_canonical(&blk.header),
    );
    cache.set_tip(point).await;
}

fn unpack_valid_transactions(
    block: BabbageBlock,
) -> impl DoubleEndedIterator<Item = (BabbageTransaction, u64)> {
    let BabbageBlock {
        header,
        transaction_bodies,
        transaction_witness_sets,
        mut auxiliary_data_set,
        invalid_transactions,
        ..
    } = block;
    let invalid_indices: HashSet<u16> = HashSet::from_iter(invalid_transactions);
    transaction_bodies
        .into_iter()
        .zip(transaction_witness_sets)
        .enumerate()
        .filter(move |(ix, _)| !invalid_indices.contains(&(*ix as u16)))
        .map(move |(ix, (tb, tw))| {
            let tx_ix = &(ix as u16);
            let tx = BabbageTransaction {
                body: tb,
                witness_set: tw,
                is_valid: true,
                auxiliary_data: auxiliary_data_set.remove(tx_ix),
                encodings: None,
            };
            (tx, header.header_body.slot)
        })
}

fn process_upstream_by_blocks<'a, Cache>(
    cache: Arc<Mutex<Cache>>,
    upgr: ChainUpgrade<BabbageBlock>,
    handle_rollbacks_after: Slot,
) -> Pin<Box<dyn Stream<Item = LedgerBlockEvent<BabbageBlock>> + 'a>>
where
    Cache: LedgerCache<BabbageBlock> + 'a,
{
    match upgr {
        ChainUpgrade::RollForward(blk) => Box::pin(stream::once(async move {
            if blk.header.header_body.slot > handle_rollbacks_after {
                cache_block(cache, blk.clone()).await;
            } else {
                cache_point(cache, &blk).await;
            }
            LedgerBlockEvent::RollForward(blk)
        })),
        ChainUpgrade::RollBackward(point) if point.get_slot() > handle_rollbacks_after => {
            Box::pin(rollback(cache, point.into()).map(LedgerBlockEvent::RollBackward))
        }
        ChainUpgrade::RollBackward(_) => {
            warn!("Node requested rollback while rollbacks are disabled.");
            Box::pin(stream::empty())
        }
    }
}

/// Handle rollback to a specific point in the past.
fn rollback<Cache>(cache: Arc<Mutex<Cache>>, to_point: Point) -> impl Stream<Item = BabbageBlock>
where
    Cache: LedgerCache<BabbageBlock>,
{
    stream! {
        loop {
            let cache = cache.lock().await;
            if let Some(tip) = cache.get_tip().await {
                if let Some(Linked(block, prev_point)) = cache.get_block(tip.clone()).await {
                    let rollback_finished = prev_point == to_point;
                    cache.delete(tip).await;
                    cache.set_tip(prev_point).await;
                    yield block;
                    if !rollback_finished {
                        continue;
                    }
                }
            }
            break;
        }
    }
}
