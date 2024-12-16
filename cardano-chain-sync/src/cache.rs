use async_std::stream::Stream;
use async_trait::async_trait;
use cml_crypto::RawBytesEncoding;
use futures::channel::mpsc;
use futures::executor::block_on;
use futures::SinkExt;
use log::trace;
use rocksdb::{Direction, IteratorMode};
use std::path::Path;
use std::sync::Arc;
use tokio::task::spawn_blocking;

use crate::client::Point;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct LinkedBlock(/*block bytes*/ pub Vec<u8>, /*prev point*/ pub Point);

pub type Inclusive<T> = T;

#[async_trait]
pub trait LedgerCache {
    async fn set_tip(&self, point: Point);
    async fn get_tip(&self) -> Option<Point>;
    async fn put_block(&self, point: Point, block: LinkedBlock);
    async fn get_block(&self, point: Point) -> Option<LinkedBlock>;
    async fn delete(&self, point: Point) -> bool;
    fn replay<'a>(&self, from_point: Inclusive<Point>) -> impl Stream<Item = LinkedBlock> + Send + 'a;
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

#[async_trait]
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
                point_key(POINT_PREFIX, &point),
                bincode::serialize(&block).unwrap(),
            )
            .unwrap()
        })
        .await
        .unwrap();
    }

    async fn get_block(&self, point: Point) -> Option<LinkedBlock> {
        let db = self.db.clone();
        spawn_blocking(move || db.get(point_key(POINT_PREFIX, &point)).unwrap())
            .await
            .unwrap()
            .and_then(|raw| bincode::deserialize(raw.as_ref()).ok())
    }

    async fn delete(&self, point: Point) -> bool {
        let db = self.db.clone();
        spawn_blocking(move || db.delete(point_key(POINT_PREFIX, &point)).unwrap())
            .await
            .unwrap();
        true
    }

    fn replay<'a>(&self, from_point: Inclusive<Point>) -> impl Stream<Item = LinkedBlock> + Send + 'a {
        let db = self.db.clone();
        let (mut snd, recv) = mpsc::unbounded();
        spawn_blocking(move || {
            trace!("Replaying blocks from point {:?}", from_point);
            let key = point_key(POINT_PREFIX, &from_point);
            let iter = db.iterator(IteratorMode::From(&key, Direction::Forward));
            let mut counter = 0;
            for item in iter {
                if let Some(blk) = item
                    .ok()
                    .and_then(|(_, raw_blk)| bincode::deserialize::<LinkedBlock>(raw_blk.as_ref()).ok())
                {
                    counter += 1;
                    block_on(snd.send(blk)).unwrap();
                }
            }
            trace!("{} blocks replayed", counter);
        });
        recv
    }
}

fn point_key(prefix: &str, point: &Point) -> Vec<u8> {
    let mut key_bytes = Vec::from(prefix.as_bytes());
    let (slot, hash) = match point {
        Point::Origin => (0, vec![]),
        Point::Specific(pt, hash) => (*pt, Vec::from(hash.to_raw_bytes())),
    };
    key_bytes.extend_from_slice(&slot.to_be_bytes());
    key_bytes.extend_from_slice(&hash);
    key_bytes
}
