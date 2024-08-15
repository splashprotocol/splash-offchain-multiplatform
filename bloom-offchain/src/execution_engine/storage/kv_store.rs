use crate::display::display_option;
use log::trace;
use spectrum_offchain::data::Has;
use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;
use std::marker::PhantomData;

pub trait KvStore<K, V> {
    fn insert(&mut self, key: K, value: V) -> Option<V>;
    fn get(&self, key: K) -> Option<V>;
    fn remove(&mut self, key: K) -> Option<V>;
}

#[derive(Debug, Clone)]
pub struct InMemoryKvStore<K, V>(HashMap<K, V>);

impl<StableId, Src> InMemoryKvStore<StableId, Src> {
    pub fn new() -> Self {
        Self(Default::default())
    }

    pub fn with_tracing() -> KvStoreWithTracing<Self> {
        KvStoreWithTracing(Self::new())
    }
}

impl<K, V> KvStore<K, V> for InMemoryKvStore<K, V>
where
    K: Eq + Hash,
    V: Clone,
{
    fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.0.insert(key, value)
    }

    fn get(&self, key: K) -> Option<V> {
        self.0.get(&key).cloned()
    }

    fn remove(&mut self, key: K) -> Option<V> {
        self.0.remove(&key)
    }
}

#[derive(Clone)]
pub struct KvStoreWithTracing<In>(In);

impl<K, V, In> KvStore<K, V> for KvStoreWithTracing<In>
where
    In: KvStore<K, V>,
    K: Copy + Display,
    V: Display,
{
    fn insert(&mut self, key: K, value: V) -> Option<V> {
        trace!("KvStore::insert(key: {}, value: {})", key, value);
        self.0.insert(key, value)
    }

    fn get(&self, key: K) -> Option<V> {
        let res = self.0.get(key);
        trace!("KvStore::get(key: {}) -> {}", key, display_option(&res));
        res
    }

    fn remove(&mut self, key: K) -> Option<V> {
        let res = self.0.remove(key);
        trace!("KvStore::remove(key: {}) -> {}", key, display_option(&res));
        res
    }
}
