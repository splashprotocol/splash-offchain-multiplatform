use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::time::{Duration, SystemTime};

use log::trace;

pub trait KvIndex<K, V> {
    fn put(&mut self, id: K, value: V);
    fn get(&self, id: &K) -> Option<V>;
    fn exists(&self, id: &K) -> bool;
    /// Mark an entry identified by the given [T::TOrderId] as subject for future eviction
    fn register_for_eviction(&mut self, id: K);
    /// Evict outdated entries.
    fn run_eviction(&mut self);
}

#[derive(Clone)]
pub struct InMemoryKvIndex<K, V> {
    store: HashMap<K, V>,
    eviction_queue: VecDeque<(SystemTime, K)>,
    eviction_delay: Duration,
}

impl<K, V> InMemoryKvIndex<K, V> {
    pub fn new(eviction_delay: Duration) -> Self {
        Self {
            store: Default::default(),
            eviction_queue: Default::default(),
            eviction_delay,
        }
    }
    pub fn with_tracing(self, tag: &str) -> OrderIndexTracing<Self> {
        OrderIndexTracing::attach(self, tag)
    }
}

impl<K, V> KvIndex<K, V> for InMemoryKvIndex<K, V>
where
    K: Eq + Hash + Display,
    V: Clone,
{
    fn put(&mut self, id: K, value: V) {
        self.store.insert(id, value);
    }

    fn get(&self, id: &K) -> Option<V> {
        self.store.get(&id).cloned()
    }

    fn exists(&self, id: &K) -> bool {
        self.store.contains_key(&id)
    }

    fn register_for_eviction(&mut self, id: K) {
        let now = SystemTime::now();
        self.eviction_queue.push_back((now + self.eviction_delay, id));
    }

    fn run_eviction(&mut self) {
        let now = SystemTime::now();
        loop {
            match self.eviction_queue.pop_front() {
                Some((ts, v)) if ts <= now => {
                    self.store.remove(&v);
                    continue;
                }
                Some((ts, v)) => self.eviction_queue.push_front((ts, v)),
                _ => {}
            }
            break;
        }
    }
}

pub struct OrderIndexTracing<R> {
    inner: R,
    tag: String,
}

impl<R> OrderIndexTracing<R> {
    pub fn attach(repo: R, tag: &str) -> Self {
        Self {
            inner: repo,
            tag: String::from(tag),
        }
    }
}

impl<K, V, R> KvIndex<K, V> for OrderIndexTracing<R>
where
    K: Display,
    V: Debug,
    R: KvIndex<K, V>,
{
    fn put(&mut self, id: K, value: V) {
        trace!(target: "offchain", "KvIndex[{}]::put({:?})", self.tag, value);
        self.inner.put(id, value)
    }

    fn get(&self, id: &K) -> Option<V> {
        trace!(target: "offchain", "KvIndex[{}]::get_state({})", self.tag, id);
        self.inner.get(id)
    }

    fn exists(&self, id: &K) -> bool {
        let res = self.inner.exists(id);
        trace!(target: "offchain", "KvIndex[{}]::exists({}) -> {}", self.tag, id, res);
        res
    }

    fn register_for_eviction(&mut self, id: K) {
        trace!(target: "offchain", "KvIndex[{}]::register_for_eviction({})", self.tag, id);
        self.inner.register_for_eviction(id)
    }

    fn run_eviction(&mut self) {
        trace!(target: "offchain", "KvIndex[{}]::run_eviction()", self.tag);
        self.inner.run_eviction()
    }
}
