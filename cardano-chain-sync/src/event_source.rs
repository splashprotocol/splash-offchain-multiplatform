use std::collections::HashSet;
use std::iter;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_stream::stream;
use cml_chain::block::Block;
use cml_chain::transaction::TransactionOutput::ConwayFormatTxOut as OtherConwayFormatTxOut;
use cml_chain::transaction::{ConwayFormatTxOut, Transaction, TransactionBody};
use cml_core::serialization::Deserialize;
use cml_core::Slot;
use cml_multi_era::babbage::{BabbageBlock, BabbageTransaction, BabbageTransactionBody};
use cml_multi_era::{MultiEraBlock, MultiEraTransactionBody};
use either::Either;
use futures::stream::StreamExt;
use futures::{stream, Stream};
use log::{info, trace, warn};
use tokio::sync::Mutex;

use spectrum_cardano_lib::hash::{hash_block_header_canonical, hash_block_header_canonical_multi_era};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;

use crate::cache::{LedgerCache, LinkedBlock};
use crate::client::Point;
use crate::data::{ChainUpgrade, LedgerBlockEvent, LedgerTxEvent};

/// Stream ledger updates as individual transactions.
pub async fn ledger_transactions<'a, S, Cache>(
    cache: Arc<Mutex<Cache>>,
    upstream: S,
    // Rollbacks will not be handled until the specified slot is reached.
    handle_rollbacks_after: Slot,
    // Reapply known blocks before pulling new ones.
    replay_from: Option<Point>,
    rollback_in_progress: Arc<AtomicBool>,
) -> impl Stream<Item = LedgerTxEvent<Either<BabbageTransaction, Transaction>>> + 'a
where
    S: Stream<Item = ChainUpgrade<MultiEraBlock>> + 'a,
    Cache: LedgerCache + 'a,
{
    let raw_replayed_blocks = match replay_from {
        None => stream::empty().boxed(),
        Some(replay_from_point) => {
            let cache = cache.lock().await;
            cache.replay(replay_from_point).boxed()
        }
    };
    let replayed_blocks = raw_replayed_blocks
        .map(|LinkedBlock(raw_blk, _)| {
            MultiEraBlock::from_cbor_bytes(&raw_blk)
                .ok()
                .map(|blk| ChainUpgrade::RollForward {
                    blk,
                    blk_bytes: raw_blk,
                    replayed: true,
                })
        })
        .filter_map(|result| async { result });
    replayed_blocks
        .chain(upstream)
        .then(move |u| {
            process_upstream_by_txs(
                Arc::clone(&cache),
                u,
                handle_rollbacks_after,
                rollback_in_progress.clone(),
            )
        })
        .flatten()
}

/// Stream ledger updates as blocks.
pub fn ledger_blocks<'a, S, Cache>(
    cache: Arc<Mutex<Cache>>,
    upstream: S,
    // Rollbacks will not be handled until the specified slot is reached.
    handle_rollbacks_after: Slot,
    rollback_in_progress: Arc<AtomicBool>,
) -> impl Stream<Item = LedgerBlockEvent<MultiEraBlock>> + 'a
where
    S: Stream<Item = ChainUpgrade<MultiEraBlock>> + 'a,
    Cache: LedgerCache + 'a,
{
    upstream.flat_map(move |u| {
        process_upstream_by_blocks(
            Arc::clone(&cache),
            u,
            handle_rollbacks_after,
            rollback_in_progress.clone(),
        )
    })
}

async fn process_upstream_by_txs<'a, Cache>(
    cache: Arc<Mutex<Cache>>,
    upgr: ChainUpgrade<MultiEraBlock>,
    handle_rollbacks_after: Slot,
    rollback_in_progress: Arc<AtomicBool>,
) -> Pin<Box<dyn Stream<Item = LedgerTxEvent<Either<BabbageTransaction, Transaction>>> + 'a>>
where
    Cache: LedgerCache + 'a,
{
    match upgr {
        ChainUpgrade::RollForward {
            blk,
            blk_bytes,
            replayed,
        } => {
            if !replayed {
                if blk.header().slot() > handle_rollbacks_after {
                    cache_block(cache, &blk, blk_bytes).await;
                } else {
                    cache_point(cache, &blk).await;
                }
            }
            info!(
                "Scanning Block {}",
                hash_block_header_canonical_multi_era(&blk.header()).to_hex()
            );
            let applied_txs: Vec<_> = unpack_valid_transactions_multi_era(blk)
                .into_iter()
                .map(|(tx, slot)| LedgerTxEvent::TxApplied { tx, slot })
                .collect();
            Box::pin(stream::iter(applied_txs))
        }
        ChainUpgrade::RollBackward(point) if point.get_slot() > handle_rollbacks_after => {
            info!("Node requested rollback to point {:?}", point);
            Box::pin(
                rollback(cache, point.into(), rollback_in_progress).flat_map(|blk| {
                    let unapplied_txs: Vec<_> = unpack_valid_transactions_multi_era(blk)
                        .into_iter()
                        .map(|(tx, _)| LedgerTxEvent::TxUnapplied(tx))
                        .rev()
                        .collect();
                    stream::iter(unapplied_txs)
                }),
            )
        }
        ChainUpgrade::RollBackward(_) => {
            warn!("Node requested rollback while rollbacks are disabled.");
            Box::pin(stream::empty())
        }
    }
}

async fn cache_block<Cache: LedgerCache>(cache: Arc<Mutex<Cache>>, blk: &MultiEraBlock, blk_bytes: Vec<u8>) {
    let cache = cache.lock().await;
    let point = Point::Specific(
        blk.header().slot(),
        hash_block_header_canonical_multi_era(&blk.header()),
    );
    let prev_point = cache.get_tip().await.unwrap_or(Point::Origin);
    cache.set_tip(point).await;
    cache.put_block(point, LinkedBlock(blk_bytes, prev_point)).await;
}

async fn cache_point<Cache: LedgerCache>(cache: Arc<Mutex<Cache>>, blk: &MultiEraBlock) {
    let cache = cache.lock().await;
    let point = Point::Specific(
        blk.header().slot(),
        hash_block_header_canonical_multi_era(&blk.header()),
    );
    cache.set_tip(point).await;
}

fn unpack_valid_transactions_multi_era(
    block: MultiEraBlock,
) -> Vec<(Either<BabbageTransaction, Transaction>, u64)> {
    match block {
        MultiEraBlock::Babbage(blk) => unpack_valid_transactions_babbage(blk)
            .map(|(tx, ix)| (Either::Left(tx), ix))
            .collect(),
        MultiEraBlock::Conway(blk) => unpack_valid_transactions_conway(blk)
            .map(|(tx, ix)| (Either::Right(tx), ix))
            .collect(),
        _ => vec![],
    }
}

fn unpack_valid_transactions_babbage(
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

fn unpack_valid_transactions_conway(block: Block) -> impl DoubleEndedIterator<Item = (Transaction, u64)> {
    let Block {
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
            let tx = Transaction {
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
    upgr: ChainUpgrade<MultiEraBlock>,
    handle_rollbacks_after: Slot,
    rollback_in_progress: Arc<AtomicBool>,
) -> Pin<Box<dyn Stream<Item = LedgerBlockEvent<MultiEraBlock>> + 'a>>
where
    Cache: LedgerCache + 'a,
{
    match upgr {
        ChainUpgrade::RollForward {
            blk,
            blk_bytes,
            replayed,
        } => Box::pin(stream::once(async move {
            if !replayed {
                if blk.header().slot() > handle_rollbacks_after {
                    cache_block(cache, &blk, blk_bytes).await;
                } else {
                    cache_point(cache, &blk).await;
                }
            }
            LedgerBlockEvent::RollForward(blk)
        })),
        ChainUpgrade::RollBackward(point) if point.get_slot() > handle_rollbacks_after => Box::pin(
            rollback(cache, point.into(), rollback_in_progress.clone()).map(LedgerBlockEvent::RollBackward),
        ),
        ChainUpgrade::RollBackward(_) => {
            warn!("Node requested rollback while rollbacks are disabled.");
            Box::pin(stream::empty())
        }
    }
}

/// Handle rollback to a specific point in the past.
fn rollback<Cache>(
    cache: Arc<Mutex<Cache>>,
    to_point: Point,
    rollback_in_progress: Arc<AtomicBool>,
) -> impl Stream<Item = MultiEraBlock>
where
    Cache: LedgerCache,
{
    stream! {
        loop {
            rollback_in_progress.swap(true, Ordering::Relaxed);
            let cache = cache.lock().await;
            if let Some(tip) = cache.get_tip().await {
                let rollback_finished = tip == to_point;
                if !rollback_finished {
                    if let Some(LinkedBlock(block_bytes, prev_point)) = cache.get_block(tip.clone()).await {
                        cache.delete(tip).await;
                        cache.set_tip(prev_point).await;
                        let block = MultiEraBlock::from_cbor_bytes(&block_bytes).expect("Block deserialization failed");
                        yield block;
                        continue;
                    }
                }
            }
            info!("Rolled back to point {:?}", to_point);
            rollback_in_progress.swap(false, Ordering::Relaxed);
            break;
        }
    }
}
