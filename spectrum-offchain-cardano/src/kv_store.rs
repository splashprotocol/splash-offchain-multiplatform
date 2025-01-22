use async_std::task::spawn_blocking;
use async_trait::async_trait;
use spectrum_offchain::kv_store::KvStore;
use spectrum_offchain::persistent_index::PersistentIndex;
use std::sync::Arc;

#[derive(Clone)]
pub struct KVIndexRocksDBViaCML {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl KVIndexRocksDBViaCML {
    pub fn new(db_path: String) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(db_path).unwrap()),
        }
    }
}

#[async_trait]
impl<K, V> PersistentIndex<K, V> for KVIndexRocksDBViaCML
where
    K: cml_core::serialization::RawBytesEncoding + Send + 'static,
    V: cml_core::serialization::Serialize + cml_core::serialization::Deserialize + Send + 'static,
    Self: Send,
{
    async fn insert(&self, key: K, value: V) -> Option<V> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let key = key.to_raw_bytes();
            let old_value = if let Some(old_value_bytes) = db.get(&key).unwrap() {
                let old_value = V::from_cbor_bytes(&*old_value_bytes).unwrap();
                Some(old_value)
            } else {
                None
            };
            tx.put(key, value.to_cbor_bytes()).unwrap();
            tx.commit().unwrap();
            old_value
        })
        .await
    }

    async fn get(&self, key: K) -> Option<V> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let key = key.to_raw_bytes();
            db.get(&key).unwrap().map(|v| V::from_cbor_bytes(&*v).unwrap())
        })
        .await
    }

    async fn remove(&self, key: K) -> Option<V> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let key = key.to_raw_bytes();
            let old_value = if let Some(old_value_bytes) = db.get(&key).unwrap() {
                let old_value = V::from_cbor_bytes(&*old_value_bytes).unwrap();
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
