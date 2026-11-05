use std::collections::HashSet;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_primitives::beacon::Beacon;
use async_stream::stream;
use cml_chain::block::Block;
use cml_chain::transaction::Transaction;
use cml_core::serialization::Deserialize;
use cml_core::Slot;
use cml_crypto::BlockHeaderHash;
use cml_multi_era::babbage::{BabbageBlock, BabbageTransaction};
use cml_multi_era::utils::MultiEraBlockHeader;
use cml_multi_era::MultiEraBlock;
use either::Either;
use futures::stream::StreamExt;
use futures::{stream, Stream};
use log::{info, warn};
use spectrum_cardano_lib::hash::{hash_block_header_canonical, hash_block_header_canonical_multi_era};
use tokio::sync::Mutex;

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
    rollback_in_progress: Beacon,
) -> impl Stream<Item = LedgerTxEvent<Either<BabbageTransaction, Transaction>>> + Send + 'a
where
    S: Stream<Item = ChainUpgrade<MultiEraBlock>> + Send + 'a,
    Cache: LedgerCache + Send + 'a,
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
    rollback_in_progress: Beacon,
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
    upgrade: ChainUpgrade<MultiEraBlock>,
    handle_rollbacks_after: Slot,
    rollback_in_progress: Beacon,
) -> Pin<Box<dyn Stream<Item = LedgerTxEvent<Either<BabbageTransaction, Transaction>>> + Send + 'a>>
where
    Cache: LedgerCache + Send + 'a,
{
    match upgrade {
        ChainUpgrade::RollForward {
            blk,
            blk_bytes,
            replayed,
        } => {
            if !replayed {
                if blk.header().slot() > handle_rollbacks_after {
                    cache_block(cache, &blk.header(), blk_bytes).await;
                } else {
                    cache_point(cache, &blk.header()).await;
                }
            }
            info!(
                "Scanning Block {}",
                hash_block_header_canonical_multi_era(&blk.header()).to_hex()
            );
            let applied_txs: Vec<_> = unpack_valid_transactions_multi_era(blk)
                .into_iter()
                .map(|(tx, slot, block_number, block_hash)| LedgerTxEvent::TxApplied {
                    tx,
                    slot,
                    block_number,
                    block_hash,
                })
                .collect();
            Box::pin(stream::iter(applied_txs))
        }
        ChainUpgrade::RollBackward(point) if point.get_slot() > handle_rollbacks_after => {
            info!("Node requested rollback to point {:?}", point);
            Box::pin(
                rollback(cache, point.into(), rollback_in_progress).flat_map(|blk| {
                    let unapplied_txs: Vec<_> = unpack_valid_transactions_multi_era(blk)
                        .into_iter()
                        .map(
                            |(tx, slot, block_number, block_hash)| LedgerTxEvent::TxUnapplied {
                                tx,
                                slot,
                                block_number,
                                block_hash,
                            },
                        )
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

async fn cache_block<Cache: LedgerCache>(
    cache: Arc<Mutex<Cache>>,
    hdr: &MultiEraBlockHeader,
    blk_bytes: Vec<u8>,
) {
    let cache = cache.lock().await;
    let point = Point::Specific(hdr.slot(), hash_block_header_canonical_multi_era(&hdr));
    let prev_point = cache.get_tip().await.unwrap_or(Point::Origin);
    cache.set_tip(point).await;
    cache.put_block(point, LinkedBlock(blk_bytes, prev_point)).await;
}

async fn cache_point<Cache: LedgerCache>(cache: Arc<Mutex<Cache>>, hdr: &MultiEraBlockHeader) {
    let cache = cache.lock().await;
    let point = Point::Specific(hdr.slot(), hash_block_header_canonical_multi_era(&hdr));
    cache.set_tip(point).await;
}

pub(crate) fn unpack_valid_transactions_multi_era(
    block: MultiEraBlock,
) -> Vec<(Either<BabbageTransaction, Transaction>, u64, u64, BlockHeaderHash)> {
    unpack_transactions_multi_era(block, false)
}

/// Unpack the transactions of a block.
///
/// With `keep_invalid == false` the phase-2 invalid transactions (the indices
/// in `invalid_transactions`) are dropped, and each kept tx has `is_valid:
/// true`. With `keep_invalid == true` every transaction of the block is kept,
/// in block order, and `is_valid` tells which ones the chain applied as
/// phase-2 invalid. The chain applies an invalid tx by its collateral only:
/// its collateral inputs are spent, its collateral return (if any) is the
/// output at index `len(outputs)`, its other inputs stay unspent and its other
/// outputs do not exist. A consumer that keeps invalid txs must apply them so.
pub fn unpack_transactions_multi_era(
    block: MultiEraBlock,
    keep_invalid: bool,
) -> Vec<(Either<BabbageTransaction, Transaction>, u64, u64, BlockHeaderHash)> {
    match block {
        MultiEraBlock::Babbage(blk) => unpack_transactions_babbage(blk, keep_invalid)
            .map(|(tx, slot, block_number, block_hash)| (Either::Left(tx), slot, block_number, block_hash))
            .collect(),
        MultiEraBlock::Conway(blk) => unpack_transactions_conway(blk, keep_invalid)
            .map(|(tx, slot, block_number, block_hash)| (Either::Right(tx), slot, block_number, block_hash))
            .collect(),
        _ => vec![],
    }
}

fn unpack_transactions_babbage(
    block: BabbageBlock,
    keep_invalid: bool,
) -> impl DoubleEndedIterator<Item = (BabbageTransaction, u64, u64, BlockHeaderHash)> {
    let block_hash = hash_block_header_canonical(&block);
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
        .map(move |(ix, (tb, tw))| {
            let tx_ix = ix as u16;
            let is_valid = !invalid_indices.contains(&tx_ix);
            (tx_ix, is_valid, tb, tw)
        })
        .filter(move |(_, is_valid, _, _)| keep_invalid || *is_valid)
        .map(move |(tx_ix, is_valid, tb, tw)| {
            let tx = BabbageTransaction {
                body: tb,
                witness_set: tw,
                is_valid,
                auxiliary_data: auxiliary_data_set.remove(&tx_ix),
                encodings: None,
            };
            (
                tx,
                header.header_body.slot,
                header.header_body.block_number,
                block_hash,
            )
        })
}

fn unpack_transactions_conway(
    block: Block,
    keep_invalid: bool,
) -> impl DoubleEndedIterator<Item = (Transaction, u64, u64, BlockHeaderHash)> {
    let block_hash = hash_block_header_canonical(&block);
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
        .map(move |(ix, (tb, tw))| {
            let tx_ix = ix as u16;
            let is_valid = !invalid_indices.contains(&tx_ix);
            (tx_ix, is_valid, tb, tw)
        })
        .filter(move |(_, is_valid, _, _)| keep_invalid || *is_valid)
        .map(move |(tx_ix, is_valid, tb, tw)| {
            let tx = Transaction {
                body: tb,
                witness_set: tw,
                is_valid,
                auxiliary_data: auxiliary_data_set.remove(&tx_ix),
                encodings: None,
            };
            (
                tx,
                header.header_body.slot,
                header.header_body.block_number,
                block_hash,
            )
        })
}

fn process_upstream_by_blocks<'a, Cache>(
    cache: Arc<Mutex<Cache>>,
    upgr: ChainUpgrade<MultiEraBlock>,
    handle_rollbacks_after: Slot,
    rollback_in_progress: Beacon,
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
                    cache_block(cache, &blk.header(), blk_bytes).await;
                } else {
                    cache_point(cache, &blk.header()).await;
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
    rollback_in_progress: Beacon,
) -> impl Stream<Item = MultiEraBlock>
where
    Cache: LedgerCache,
{
    stream! {
        loop {
            rollback_in_progress.alter(true);
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
            rollback_in_progress.alter(false);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::block::{Block, Header, HeaderBody, OperationalCert, ProtocolVersion};
    use cml_chain::crypto::{KESSignature, VRFCert, Vkey};
    use cml_chain::transaction::{TransactionBody, TransactionWitnessSet};
    use cml_core::ordered_hash_map::OrderedHashMap;
    use cml_crypto::{BlockBodyHash, Ed25519Signature, KESVkey, RawBytesEncoding, VRFVkey};
    use cml_multi_era::MultiEraBlock;

    use super::unpack_transactions_multi_era;

    /// A Conway block with three txs; the middle one (index 1) is phase-2
    /// invalid. Each tx is told apart by its fee.
    fn block_with_invalid_middle_tx() -> MultiEraBlock {
        let header_body = HeaderBody::new(
            14148288,
            1000,
            None,
            Vkey::from_raw_bytes(&[1u8; 32]).unwrap(),
            VRFVkey::from_raw_bytes(&[2u8; 32]).unwrap(),
            VRFCert::new(vec![3u8; 64], vec![4u8; 80]).unwrap(),
            0,
            BlockBodyHash::from_raw_bytes(&[5u8; 32]).unwrap(),
            OperationalCert::new(
                KESVkey::from_raw_bytes(&[6u8; 32]).unwrap(),
                0,
                0,
                Ed25519Signature::from_raw_bytes(&[7u8; 64]).unwrap(),
            ),
            ProtocolVersion::new(10, 0),
        );
        let header = Header::new(header_body, KESSignature::new(vec![8u8; 448]).unwrap());
        let bodies: Vec<TransactionBody> = [100u64, 200, 300]
            .into_iter()
            .map(|fee| TransactionBody::new(vec![].into(), vec![], fee))
            .collect();
        let witnesses = vec![TransactionWitnessSet::new(); 3];
        MultiEraBlock::Conway(Block::new(
            header,
            bodies,
            witnesses,
            OrderedHashMap::new(),
            vec![1],
        ))
    }

    fn fees_and_validity(keep_invalid: bool) -> Vec<(u64, bool)> {
        unpack_transactions_multi_era(block_with_invalid_middle_tx(), keep_invalid)
            .into_iter()
            .map(|(tx, slot, block_number, _)| {
                assert_eq!((slot, block_number), (1000, 14148288));
                let tx = tx.right().expect("a Conway tx");
                (tx.body.fee, tx.is_valid)
            })
            .collect()
    }

    #[test]
    fn the_valid_only_unpack_drops_the_invalid_tx() {
        assert_eq!(fees_and_validity(false), vec![(100, true), (300, true)]);
    }

    #[test]
    fn the_full_unpack_keeps_the_invalid_tx_in_block_order_with_its_flag() {
        assert_eq!(
            fees_and_validity(true),
            vec![(100, true), (200, false), (300, true)]
        );
    }
}
