use crate::store::{ExecutionIndex, IndexSnapshot};
use async_trait::async_trait;
use rocksdb::{Options, DB};
use std::path::Path;
use std::sync::Arc;
use tokio::task::spawn_blocking;

const SNAPSHOT_KEY: &[u8] = b"snapshot";

#[derive(Clone)]
pub struct RocksIndex {
    db: Arc<DB>,
}

impl RocksIndex {
    pub fn open<P: AsRef<Path>>(path: P) -> Self {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        Self {
            db: Arc::new(DB::open(&opts, path).expect("failed to open execution index RocksDB")),
        }
    }
}

#[async_trait]
impl ExecutionIndex for RocksIndex {
    async fn snapshot(&self) -> IndexSnapshot {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || {
            db.get(SNAPSHOT_KEY)
                .expect("failed to read execution index snapshot")
                .map(|bytes| {
                    rmp_serde::from_slice(&bytes).expect("failed to decode execution index snapshot")
                })
                .unwrap_or_default()
        })
        .await
        .expect("snapshot task failed")
    }

    async fn put_snapshot(&self, snapshot: IndexSnapshot) {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || {
            let bytes = rmp_serde::to_vec(&snapshot).expect("failed to encode execution index snapshot");
            db.put(SNAPSHOT_KEY, bytes)
                .expect("failed to write execution index snapshot");
        })
        .await
        .expect("put snapshot task failed");
    }
}
