use async_std::task::spawn_blocking;
use async_trait::async_trait;
use serde::Serialize;
use spectrum_offchain::kv_store::KvStore;
use spectrum_offchain::persistent_index::PersistentIndex;
use std::sync::Arc;

#[derive(Clone)]
pub struct IndexRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl IndexRocksDB {
    pub fn new(db_path: String) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(db_path).unwrap()),
        }
    }
}

#[async_trait]
impl<K, V> PersistentIndex<K, V> for IndexRocksDB
where
    K: Serialize + Send + 'static,
    V: cml_core::serialization::Serialize + cml_core::serialization::Deserialize + Send + 'static,
    Self: Send,
{
    async fn insert(&self, key: K, value: V) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let key = rmp_serde::to_vec(&key).unwrap();
            tx.put(key, value.to_cbor_bytes()).unwrap();
            tx.commit().unwrap();
        })
        .await
    }

    async fn get(&self, key: K) -> Option<V> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let key = rmp_serde::to_vec(&key).unwrap();
            db.get(&key).unwrap().map(|v| V::from_cbor_bytes(&*v).unwrap())
        })
        .await
    }

    async fn remove(&self, key: K) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let key = rmp_serde::to_vec(&key).unwrap();
            db.delete(&key).unwrap();
        })
        .await
    }
}
