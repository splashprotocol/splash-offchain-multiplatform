use async_trait::async_trait;
use log::trace;
use std::fmt::Display;

use crate::display::display_option;

#[async_trait]
pub trait PersistentIndex<K, V> {
    async fn insert(&self, key: K, value: V);
    async fn get(&self, key: K) -> Option<V>;
    async fn remove(&self, key: K);
}

#[derive(Clone)]
pub struct PersistentIndexWithTracing<In>(In);

#[async_trait]
impl<K, V, In> PersistentIndex<K, V> for PersistentIndexWithTracing<In>
where
    In: PersistentIndex<K, V>,
    K: Copy + Display + Send + 'static,
    V: Display + Send + 'static,
    Self: Send + Sync,
{
    async fn insert(&self, key: K, value: V) {
        trace!("PersistentIndex::insert(key: {}, value: {})", key, value);
        self.0.insert(key, value).await
    }

    async fn get(&self, key: K) -> Option<V> {
        let res = self.0.get(key).await;
        trace!("PersistentIndex::get(key: {}) -> {}", key, display_option(&res));
        res
    }

    async fn remove(&self, key: K) {
        trace!("PersistentIndex::remove(key: {})", key);
        self.0.remove(key).await;
    }
}
