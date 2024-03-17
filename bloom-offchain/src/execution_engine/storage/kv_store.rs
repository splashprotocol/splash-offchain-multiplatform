use std::collections::HashMap;
use std::hash::Hash;

pub trait KvStore<K, V> {
    fn insert(&mut self, key: K, value: V) -> Option<V>;
    fn get(&self, key: K) -> Option<V>;
}

#[derive(Debug, Clone)]
pub struct InMemoryKvStore<K, V>(HashMap<K, V>);

impl<StableId, Src> InMemoryKvStore<StableId, Src> {
    pub fn new() -> Self {
        Self(Default::default())
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
}
