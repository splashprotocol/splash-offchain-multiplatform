use std::sync::Arc;

use async_std::task::spawn_blocking;
use async_trait::async_trait;
use ergo_lib::{
    chain::transaction::{Transaction, TxId},
    ergo_chain_types::BlockId,
    ergotree_ir::serialization::SigmaSerializable,
};

use crate::constants::ERGO_MAX_ROLLBACK_DEPTH;
use crate::model::{Block, BlockRecord};
use crate::rocksdb::RocksConfig;

use super::chain_cache::ChainCache;

static BEST_BLOCK: &str = "BEST_BLOCK";
static OLDEST_BLOCK: &str = "OLDEST_BLOCK";

/// Given a block `B`, let `HB` denote the (lowercase) hex-representation of block's ID. Then
///  - {HB}:p is the key which maps to the hex-representation of B's parent block ID.
///  - {HB}:c is the key which maps to the hex-representation of B's child block ID, if it currently
///    exists.
///  - {HB}:h is the key which maps to the height of `B`.
///  - {HB}:t is the key which maps to a binary-encoding of a Vec containing the hex-representation
///    `HT` of the transaction ID of every transaction of `B`.
///    - Every {HT} is a key which maps to the Ergo-binary-encoded representation of its
///      transaction.
///  - {BEST_BLOCK} is a key which maps to a `BlockRecord` instance associated with the most
///    recently-stored block.
///  - {OLDEST_BLOCK} is a key which maps to a `BlockRecord` instance associated with the oldest
///    block in the persistent store.
pub struct ChainCacheRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
    /// Represents the maximum number of blocks in the persistent store.
    pub max_rollback_depth: u32,
}

impl ChainCacheRocksDB {
    pub fn new(conf: RocksConfig) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(conf.db_path).unwrap()),
            max_rollback_depth: ERGO_MAX_ROLLBACK_DEPTH,
        }
    }
}

/// The Rocksdb bindings are not async, so we must wrap any uses of the library in
/// `async_std::task::spawn_blocking`.
#[async_trait]
impl ChainCache for ChainCacheRocksDB {
    async fn append_block(&mut self, block: Block) {
        let db = self.db.clone();
        let max_rollback_depth = self.max_rollback_depth;
        spawn_blocking(move || {
            let oldest_block_key = bincode::serialize(OLDEST_BLOCK).unwrap();
            let db_tx = db.transaction();
            db_tx
                .put(
                    &postfixed_key(&block.id, PARENT_POSTFIX),
                    bincode::serialize(&block.parent_id).unwrap(),
                )
                .unwrap();
            db_tx
                .put(
                    &postfixed_key(&block.parent_id, CHILD_POSTFIX),
                    bincode::serialize(&block.id).unwrap(),
                )
                .unwrap();
            db_tx
                .put(
                    &postfixed_key(&block.id, HEIGHT_POSTFIX),
                    bincode::serialize(&block.height).unwrap(),
                )
                .unwrap();

            let tx_ids: Vec<TxId> = block.transactions.iter().map(|t| t.id()).collect();
            // We package together all transactions ids into a Vec.
            db_tx
                .put(
                    &postfixed_key(&block.id, TRANSACTION_POSTFIX),
                    bincode::serialize(&tx_ids).unwrap(),
                )
                .unwrap();

            // Each transaction is stored in an Ergo-serialized binary representation.
            let serialized_transactions = block
                .transactions
                .iter()
                .map(|t| t.sigma_serialize_bytes().unwrap());

            // Map each transaction id to a bincode-encoded representation of its transaction.
            for (tx_id, tx) in tx_ids.into_iter().zip(serialized_transactions) {
                db_tx.put(bincode::serialize(&tx_id).unwrap(), tx).unwrap();
            }

            db_tx
                .put(
                    bincode::serialize(BEST_BLOCK).unwrap(),
                    bincode::serialize(&BlockRecord {
                        id: block.id,
                        height: block.height,
                    })
                    .unwrap(),
                )
                .unwrap();

            if let Some(bytes) = db_tx.get(&oldest_block_key).unwrap() {
                let BlockRecord {
                    id: oldest_id,
                    height: oldest_height,
                } = bincode::deserialize(&bytes).unwrap();

                // Replace OLDEST_BLOCK if the persistent store is at capacity.
                if block.height - oldest_height == max_rollback_depth {
                    let new_oldest_block = BlockRecord {
                        id: bincode::deserialize(
                            &db_tx
                                .get(&postfixed_key(&oldest_id, CHILD_POSTFIX))
                                .unwrap()
                                .unwrap(),
                        )
                        .unwrap(),
                        height: oldest_height + 1,
                    };
                    db_tx
                        .put(
                            bincode::serialize(OLDEST_BLOCK).unwrap(),
                            bincode::serialize(&new_oldest_block).unwrap(),
                        )
                        .unwrap();

                    // Delete all data relating to the 'old' oldest block
                    let tx_ids_bytes = db_tx
                        .get(&postfixed_key(&oldest_id, TRANSACTION_POSTFIX))
                        .unwrap()
                        .unwrap();
                    let tx_ids: Vec<TxId> = bincode::deserialize(&tx_ids_bytes).unwrap();
                    for tx_id in tx_ids {
                        let tx_key = bincode::serialize(&tx_id).unwrap();
                        db_tx.delete(tx_key).unwrap();
                    }

                    db_tx
                        .delete(postfixed_key(&oldest_id, TRANSACTION_POSTFIX))
                        .unwrap();
                    db_tx.delete(postfixed_key(&oldest_id, HEIGHT_POSTFIX)).unwrap();
                    db_tx.delete(postfixed_key(&oldest_id, PARENT_POSTFIX)).unwrap();
                    db_tx.delete(postfixed_key(&oldest_id, CHILD_POSTFIX)).unwrap();
                }
            } else {
                // This is the very first block to add to the store
                db_tx
                    .put(
                        bincode::serialize(OLDEST_BLOCK).unwrap(),
                        bincode::serialize(&BlockRecord {
                            id: block.id,
                            height: block.height,
                        })
                        .unwrap(),
                    )
                    .unwrap();
            }

            db_tx.commit().unwrap();
        })
        .await
    }

    async fn exists(&mut self, block_id: BlockId) -> bool {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.get(postfixed_key(&block_id, HEIGHT_POSTFIX))
                .unwrap()
                .is_some()
        })
        .await
    }

    async fn get_best_block(&mut self) -> Option<BlockRecord> {
        let db = self.db.clone();
        spawn_blocking(move || {
            if let Ok(Some(bytes)) = db.get(bincode::serialize(BEST_BLOCK).unwrap()) {
                bincode::deserialize(&bytes).ok()
            } else {
                None
            }
        })
        .await
    }

    async fn take_best_block(&mut self) -> Option<Block> {
        let db = self.db.clone();
        spawn_blocking::<_, Option<Block>>(move || {
            let best_block_key = bincode::serialize(BEST_BLOCK).unwrap();

            loop {
                let db_tx = db.transaction();
                // The call to `get_for_update` is crucial; it plays an identical role as the WATCH
                // command in redis (refer to docs of `take_best_block` in impl of [`RedisClient`].
                if let Some(best_block_bytes) = db_tx.get_for_update(&best_block_key, true).unwrap() {
                    let BlockRecord { id, height } = bincode::deserialize(&best_block_bytes).unwrap();

                    if let Some(tx_ids_bytes) = db_tx.get(&postfixed_key(&id, TRANSACTION_POSTFIX)).unwrap() {
                        let mut transactions = vec![];
                        let tx_ids: Vec<TxId> = bincode::deserialize(&tx_ids_bytes).unwrap();
                        for tx_id in tx_ids {
                            let tx_key = bincode::serialize(&tx_id).unwrap();
                            let tx_bytes = db_tx.get(&tx_key).unwrap().unwrap();

                            // Don't need transaction anymore, delete
                            db_tx.delete(&tx_key).unwrap();

                            transactions.push(Transaction::sigma_parse_bytes(&tx_bytes).unwrap());
                        }

                        let parent_id_bytes =
                            db_tx.get(&postfixed_key(&id, PARENT_POSTFIX)).unwrap().unwrap();
                        let parent_id: BlockId = bincode::deserialize(&parent_id_bytes).unwrap();

                        db_tx.delete(&best_block_key).unwrap();

                        // The new best block will now be the parent of the old best block, if the parent
                        // exists in the cache.
                        if db_tx
                            .get(&postfixed_key(&parent_id, PARENT_POSTFIX))
                            .unwrap()
                            .is_some()
                        {
                            let parent_id_height_bytes = db_tx
                                .get(&postfixed_key(&parent_id, HEIGHT_POSTFIX))
                                .unwrap()
                                .unwrap();
                            let parent_id_height: u32 =
                                bincode::deserialize(&parent_id_height_bytes).unwrap();

                            db_tx
                                .put(
                                    &best_block_key,
                                    bincode::serialize(&BlockRecord {
                                        id: parent_id,
                                        height: parent_id_height,
                                    })
                                    .unwrap(),
                                )
                                .unwrap();
                        }
                        match db_tx.commit() {
                            Ok(_) => {
                                return Some(Block {
                                    id,
                                    parent_id,
                                    height,
                                    timestamp: 0, // todo: DEV-573
                                    transactions,
                                });
                            }
                            Err(e) => {
                                if e.kind() == rocksdb::ErrorKind::Busy {
                                    continue;
                                } else {
                                    panic!("Unexpected error: {}", e);
                                }
                            }
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
        })
        .await
    }
}

fn postfixed_key(block_id: &BlockId, s: &str) -> Vec<u8> {
    let mut bytes = bincode::serialize(block_id).unwrap();
    let p_bytes = bincode::serialize(s).unwrap();
    bytes.extend_from_slice(&p_bytes);
    bytes
}

const PARENT_POSTFIX: &str = ":p";
const CHILD_POSTFIX: &str = ":c";
const HEIGHT_POSTFIX: &str = ":h";
const TRANSACTION_POSTFIX: &str = ":t";

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_std::task::spawn_blocking;
    use chrono::Utc;
    use ergo_lib::{
        chain::transaction::Transaction,
        ergo_chain_types::{BlockId, Digest32},
    };
    use rand::RngCore;
    use sigma_test_util::force_any_val;

    use crate::{
        cache::{
            chain_cache::ChainCache,
            rocksdb::{HEIGHT_POSTFIX, TRANSACTION_POSTFIX},
        },
        model::{Block, BlockRecord},
    };

    use super::{postfixed_key, ChainCacheRocksDB, OLDEST_BLOCK, PARENT_POSTFIX};

    async fn verify_oldest_block(
        expected_block_id: BlockId,
        expected_height: u32,
        db: Arc<rocksdb::OptimisticTransactionDB>,
    ) {
        spawn_blocking::<_, ()>(move || {
            let oldest_block_key = bincode::serialize(OLDEST_BLOCK).unwrap();
            let bytes = db.get(oldest_block_key).unwrap().unwrap();
            let BlockRecord {
                id: oldest_id,
                height: oldest_height,
            } = bincode::deserialize(&bytes).unwrap();

            assert_eq!(oldest_id, expected_block_id);
            assert_eq!(oldest_height, expected_height);
            let parent_block_id_bytes = db
                .get(postfixed_key(&oldest_id, PARENT_POSTFIX))
                .unwrap()
                .unwrap();
            let parent_block_id: BlockId = bincode::deserialize(&parent_block_id_bytes).unwrap();
            assert!(db
                .get(postfixed_key(&parent_block_id, PARENT_POSTFIX))
                .unwrap()
                .is_none());
            assert!(db
                .get(postfixed_key(&parent_block_id, HEIGHT_POSTFIX))
                .unwrap()
                .is_none());
            assert!(db
                .get(postfixed_key(&parent_block_id, TRANSACTION_POSTFIX))
                .unwrap()
                .is_none());
        })
        .await;
    }

    #[tokio::test]
    async fn test_max_rollback_length() {
        let block_ids: Vec<_> = force_any_val::<[Digest32; 30]>()
            .into_iter()
            .map(BlockId)
            .collect();
        let mut height = 1;

        let mut blocks = vec![];

        let max_rollback_depth = 7;

        let rnd = rand::thread_rng().next_u32();
        let mut client = ChainCacheRocksDB {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap()),
            max_rollback_depth,
        };

        for i in 1..30 {
            let transactions = force_any_val::<[Transaction; 10]>().to_vec();
            let parent_id = block_ids[i - 1];
            let id = block_ids[i];
            let timestamp = Utc::now().timestamp() as u64;
            let block = Block {
                id,
                parent_id,
                height,
                timestamp,
                transactions,
            };
            blocks.push(block.clone());

            client.append_block(block).await;
            assert!(client.exists(id).await);
            if i < (max_rollback_depth as usize) {
                verify_oldest_block(block_ids[1], 1, client.db.clone()).await;
            } else {
                verify_oldest_block(
                    block_ids[i + 1 - (max_rollback_depth as usize)],
                    height + 1 - max_rollback_depth,
                    client.db.clone(),
                )
                .await;
            }
            height += 1;
        }
    }
}
