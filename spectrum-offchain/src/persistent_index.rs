use async_std::task::spawn_blocking;
use async_trait::async_trait;
use log::trace;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;
use std::sync::Arc;

use crate::display::display_option;

#[async_trait]
pub trait PersistentIndex<K, V> {
    async fn insert(&self, key: K, value: V) -> Option<V>;
    async fn get(&self, key: K) -> Option<V>;
    async fn remove(&self, key: K) -> Option<V>;
}

#[derive(Clone)]
pub struct KvIndexWithTracing<In>(In);

#[async_trait]
impl<K, V, In> PersistentIndex<K, V> for KvIndexWithTracing<In>
where
    In: PersistentIndex<K, V>,
    K: Copy + Display + Send + 'static,
    V: Display + Send + 'static,
    Self: Send + Sync,
{
    async fn insert(&self, key: K, value: V) -> Option<V> {
        trace!("KvIndex::insert(key: {}, value: {})", key, value);
        self.0.insert(key, value).await
    }

    async fn get(&self, key: K) -> Option<V> {
        let res = self.0.get(key).await;
        trace!("KvIndex::get(key: {}) -> {}", key, display_option(&res));
        res
    }

    async fn remove(&self, key: K) -> Option<V> {
        let res = self.0.remove(key).await;
        trace!("KvIndex::remove(key: {}) -> {}", key, display_option(&res));
        res
    }
}
