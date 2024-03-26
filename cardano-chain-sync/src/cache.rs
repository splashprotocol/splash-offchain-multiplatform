use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tokio::task::spawn_blocking;

use crate::client::Point;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct LinkedBlock(pub Vec<u8>, pub Point);

pub trait LedgerCache {
    async fn set_tip(&self, point: Point);
    async fn get_tip(&self) -> Option<Point>;
    async fn put_block(&self, point: Point, block: LinkedBlock);
    async fn get_block(&self, point: Point) -> Option<LinkedBlock>;
    async fn delete(&self, point: Point) -> bool;
}

pub struct LedgerCacheRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl LedgerCacheRocksDB {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(path).unwrap()),
        }
    }
}

const LATEST_POINT: &str = "a:";
const POINT_PREFIX: &str = "b:";

impl LedgerCache for LedgerCacheRocksDB {
    async fn set_tip(&self, point: Point) {
        let db = self.db.clone();
        spawn_blocking(move || db.put(LATEST_POINT, bincode::serialize(&point).unwrap()).unwrap())
            .await
            .unwrap();
    }

    async fn get_tip(&self) -> Option<Point> {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.get(LATEST_POINT)
                .unwrap()
                .and_then(|raw| bincode::deserialize(&*raw).ok())
        })
        .await
        .unwrap()
    }

    async fn put_block(&self, point: Point, block: LinkedBlock) {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.put(
                make_key(POINT_PREFIX, &point),
                bincode::serialize(&block).unwrap(),
            )
            .unwrap()
        })
        .await
        .unwrap();
    }

    async fn get_block(&self, point: Point) -> Option<LinkedBlock> {
        let db = self.db.clone();
        spawn_blocking(move || db.get(make_key(POINT_PREFIX, &point)).unwrap())
            .await
            .unwrap()
            .and_then(|raw| bincode::deserialize(raw.as_ref()).ok())
    }

    async fn delete(&self, point: Point) -> bool {
        let db = self.db.clone();
        spawn_blocking(move || db.delete(make_key(POINT_PREFIX, &point)).unwrap())
            .await
            .unwrap();
        true
    }
}

fn make_key<T: Serialize>(prefix: &str, id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(prefix).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}
