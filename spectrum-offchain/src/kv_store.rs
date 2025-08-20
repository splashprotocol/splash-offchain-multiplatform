use async_std::task::spawn_blocking;
use async_trait::async_trait;
use log::trace;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt::Display;
use std::sync::Arc;

use crate::display::display_option;

#[async_trait]
pub trait KvStore<K, V> {
    async fn insert(&self, key: K, value: V) -> Option<V>;
    async fn get(&self, key: K) -> Option<V>;
    async fn remove(&self, key: K) -> Option<V>;
}

#[derive(Clone)]
pub struct KvStoreWithTracing<In>(In);

#[async_trait]
impl<K, V, In> KvStore<K, V> for KvStoreWithTracing<In>
where
    In: KvStore<K, V>,
    K: Copy + Display + Send + 'static,
    V: Display + Send + 'static,
    Self: Send + Sync,
{
    async fn insert(&self, key: K, value: V) -> Option<V> {
        trace!("KvStore::insert(key: {}, value: {})", key, value);
        self.0.insert(key, value).await
    }

    async fn get(&self, key: K) -> Option<V> {
        let res = self.0.get(key).await;
        trace!("KvStore::get(key: {}) -> {}", key, display_option(&res));
        res
    }

    async fn remove(&self, key: K) -> Option<V> {
        let res = self.0.remove(key).await;
        trace!("KvStore::remove(key: {}) -> {}", key, display_option(&res));
        res
    }
}

#[derive(Clone)]
pub struct KVStoreRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl KVStoreRocksDB {
    pub fn new(db_path: String) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(db_path).unwrap()),
        }
    }
}

#[async_trait]
impl<K, V> KvStore<K, V> for KVStoreRocksDB
where
    K: Serialize + Send + 'static,
    V: Serialize + DeserializeOwned + Send + 'static,
    Self: Send,
{
    async fn insert(&self, key: K, value: V) -> Option<V> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let key = rmp_serde::to_vec(&key).unwrap();
            let old_value = if let Some(old_value_bytes) = db.get(&key).unwrap() {
                let old_value: V = rmp_serde::from_slice(&old_value_bytes).unwrap();
                Some(old_value)
            } else {
                None
            };
            tx.put(key, rmp_serde::to_vec_named(&value).unwrap()).unwrap();
            tx.commit().unwrap();
            old_value
        })
        .await
    }

    async fn get(&self, key: K) -> Option<V> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let key = rmp_serde::to_vec(&key).unwrap();
            db.get(&key).unwrap().map(|v| rmp_serde::from_slice(&v).unwrap())
        })
        .await
    }

    async fn remove(&self, key: K) -> Option<V> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let key = rmp_serde::to_vec(&key).unwrap();
            let old_value = if let Some(old_value_bytes) = db.get(&key).unwrap() {
                let old_value: V = rmp_serde::from_slice(&old_value_bytes).unwrap();
                Some(old_value)
            } else {
                None
            };
            db.delete(&key).unwrap();
            old_value
        })
        .await
    }
}
