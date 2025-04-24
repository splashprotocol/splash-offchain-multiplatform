use async_trait::async_trait;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use rocksdb::{IteratorMode, Transaction, TransactionDB};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait BroadcastQueue<T> {
    async fn enqueue(&self, item: T);
}

#[async_trait]
pub trait Dequeue<T> {
    async fn next(&self) -> Option<(u64, T)>;
    async fn delete(&self, seq_num: u64);
}

#[derive(Clone)]
pub struct RocksDB {
    db: Arc<TransactionDB>,
}

impl RocksDB {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            db: Arc::new(TransactionDB::open_default(path).unwrap()),
        }
    }
}

#[async_trait]
impl<T> BroadcastQueue<T> for Vec<RocksDB>
where
    T: Serialize + Send + Sync + Clone + 'static + std::fmt::Debug,
{
    async fn enqueue(&self, item: T) {
        let futures = FuturesUnordered::new();
        for db in self.iter() {
            let item = item.clone();
            let db = db.clone();
            futures.push(spawn_blocking(move || {
                let txn = db.db.transaction();
                let max_key = read_max_key(&txn);
                let next_key = serialize_seq_num(max_key + 1);
                let item_bytes = rmp_serde::to_vec_named(&item).unwrap();
                txn.put(&next_key, &item_bytes).unwrap();
                txn.commit().unwrap();
            }))
        }
        futures.collect::<Vec<_>>().await;
    }
}

#[async_trait]
impl<T> Dequeue<T> for RocksDB
where
    T: DeserializeOwned + Send + Sync + Clone + 'static,
{
    async fn next(&self) -> Option<(u64, T)> {
        let db = self.db.clone();
        spawn_blocking(move || read_min_kv(&db)).await.unwrap()
    }
    async fn delete(&self, seq_num: u64) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let key = serialize_seq_num(seq_num);
            db.delete(&key).unwrap();
        })
        .await
        .unwrap();
    }
}

pub fn read_max_key(tx: &Transaction<TransactionDB>) -> u64 {
    let mut iter = tx.iterator(IteratorMode::End);
    let mut seq_num = 0u64;
    if let Some(Ok((key, _))) = iter.next() {
        let max_seq_num = deserialize_seq_num(&key);
        seq_num = max_seq_num;
    }
    seq_num
}

pub fn read_min_kv<T: DeserializeOwned>(db: &Arc<TransactionDB>) -> Option<(u64, T)> {
    let mut iter = db.iterator(IteratorMode::Start);
    if let Some(Ok((key, value))) = iter.next() {
        let max_seq_num = deserialize_seq_num(&key);
        return Some((max_seq_num, rmp_serde::from_slice(&value).unwrap()));
    }
    None
}

fn serialize_seq_num(seq_num: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(8);
    key.extend_from_slice(&seq_num.to_be_bytes());
    key
}

fn deserialize_seq_num(key: &[u8]) -> u64 {
    let mut array = [0u8; 8];
    array.copy_from_slice(key);
    u64::from_be_bytes(array)
}

#[cfg(test)]
mod tests {
    use crate::queue::{BroadcastQueue, Dequeue, RocksDB};
    use rocksdb::{Options, SingleThreaded, TransactionDB};
    use std::path::{Path, PathBuf};

    #[tokio::test]
    async fn test_enqueue_and_dequeue() {
        let path = DBPath::new("test_rocksdb");
        let rocks_db = RocksDB::new(&path);
        let queue: Vec<_> = vec![rocks_db.clone()];

        // Enqueue integers
        let items = vec![1, 2, 3, 4, 5];
        for &item in &items {
            queue.enqueue(item).await;
        }

        // Dequeue items and verify the order
        let mut dequeued_items = vec![];
        for _ in 0..items.len() {
            if let Some((seq_num, value)) = <RocksDB as Dequeue<u64>>::next(&rocks_db).await {
                dequeued_items.push(value);
                <RocksDB as Dequeue<u64>>::delete::<'_, '_>(&rocks_db, seq_num).await;
                // Ensure the item is deleted
            }
        }

        // Verify that dequeued items match the enqueued items
        assert_eq!(dequeued_items, items);
    }

    /// Temporary database path which calls DB::Destroy when DBPath is dropped.
    pub struct DBPath {
        dir: tempfile::TempDir, // kept for cleaning up during drop
        path: PathBuf,
    }

    impl DBPath {
        /// Produces a fresh (non-existent) temporary path which will be DB::destroy'ed automatically.
        pub fn new(prefix: &str) -> DBPath {
            let dir = tempfile::Builder::new()
                .prefix(prefix)
                .tempdir()
                .expect("Failed to create temporary path for db.");
            let path = dir.path().join("db");

            DBPath { dir, path }
        }
    }

    impl Drop for DBPath {
        fn drop(&mut self) {
            let opts = Options::default();
            TransactionDB::<SingleThreaded>::destroy(&opts, &self.path)
                .expect("Failed to destroy temporary DB");
        }
    }

    /// Convert a DBPath ref to a Path ref.
    /// We don't implement this for DBPath values because we want them to
    /// exist until the end of their scope, not get passed into functions and
    /// dropped early.
    impl AsRef<Path> for &DBPath {
        fn as_ref(&self) -> &Path {
            &self.path
        }
    }
}
