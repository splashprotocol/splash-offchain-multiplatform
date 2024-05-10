use std::sync::Arc;

use async_std::task::spawn_blocking;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::backlog::data::BacklogOrder;
use crate::data::order::UniqueOrder;
use crate::rocks::RocksConfig;

#[async_trait]
pub trait BacklogStore<TOrd>
where
    TOrd: UniqueOrder,
{
    async fn put(&self, ord: BacklogOrder<TOrd>);
    async fn exists(&self, ord_id: TOrd::TOrderId) -> bool;
    async fn remove(&self, ord_id: TOrd::TOrderId);
    async fn get(&self, ord_id: TOrd::TOrderId) -> Option<BacklogOrder<TOrd>>;
    async fn find_orders<F>(&self, f: F) -> Vec<BacklogOrder<TOrd>>
    where
        F: Fn(&TOrd) -> bool + Send + 'static;
}

pub struct BacklogStoreRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl BacklogStoreRocksDB {
    pub fn new(conf: RocksConfig) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(conf.db_path).unwrap()),
        }
    }
}

#[async_trait]
impl<TOrd> BacklogStore<TOrd> for BacklogStoreRocksDB
where
    TOrd: UniqueOrder + Serialize + DeserializeOwned + Send + 'static,
    TOrd::TOrderId: Serialize + DeserializeOwned + Send,
{
    async fn put(&self, ord: BacklogOrder<TOrd>) {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.put(
                bincode::serialize(&ord.order.get_self_ref()).unwrap(),
                bincode::serialize(&ord).unwrap(),
            )
            .unwrap();
        })
        .await;
    }
    async fn exists(&self, ord_id: TOrd::TOrderId) -> bool {
        let db = self.db.clone();
        spawn_blocking(move || db.get(bincode::serialize(&ord_id).unwrap()).unwrap().is_some()).await
    }

    async fn remove(&self, ord_id: TOrd::TOrderId) {
        let db = self.db.clone();
        spawn_blocking(move || db.delete(bincode::serialize(&ord_id).unwrap()).unwrap()).await;
    }

    async fn get(&self, ord_id: TOrd::TOrderId) -> Option<BacklogOrder<TOrd>> {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.get(bincode::serialize(&ord_id).unwrap())
                .unwrap()
                .map(|b| bincode::deserialize(&b).unwrap())
        })
        .await
    }

    async fn find_orders<F>(&self, f: F) -> Vec<BacklogOrder<TOrd>>
    where
        F: Fn(&TOrd) -> bool + Send + 'static,
    {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.iterator(rocksdb::IteratorMode::Start)
                .filter_map(|i| {
                    let (_, v) = i.unwrap();
                    if let Ok(b) = bincode::deserialize::<BacklogOrder<TOrd>>(&v) {
                        if f(&b.order) {
                            return Some(b);
                        }
                    }
                    None
                })
                .collect()
        })
        .await
    }
}
