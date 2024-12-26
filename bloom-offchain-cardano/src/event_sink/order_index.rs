use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::time::{Duration, Instant};

use log::trace;
use spectrum_offchain::clock::Clock;

pub trait KvIndex<K, V> {
    fn put(&mut self, id: K, value: V);
    fn get(&self, id: &K) -> Option<V>;
    fn exists(&self, id: &K) -> bool;
    /// Register [K] for future eviction.
    fn register_for_eviction(&mut self, id: K);
    /// Evict outdated entries.
    fn run_eviction(&mut self);
}

#[derive(Clone)]
pub struct InMemoryKvIndex<K, V, C> {
    store: HashMap<K, V>,
    eviction_queue: VecDeque<(Instant, K)>,
    eviction_delay: Duration,
    clock: C,
}

impl<K, V, C> InMemoryKvIndex<K, V, C> {
    pub fn new(eviction_delay: Duration, clock: C) -> Self {
        Self {
            store: Default::default(),
            eviction_queue: Default::default(),
            eviction_delay,
            clock,
        }
    }
    pub fn with_tracing(self, tag: &str) -> OrderIndexTracing<Self> {
        OrderIndexTracing::attach(self, tag)
    }
}

impl<K, V, C> KvIndex<K, V> for InMemoryKvIndex<K, V, C>
where
    K: Eq + Hash + Display,
    V: Clone,
    C: Clock,
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
        let now = self.clock.current_time();
        self.eviction_queue.push_back((now + self.eviction_delay, id));
    }

    fn run_eviction(&mut self) {
        let now = self.clock.current_time();
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

#[derive(Clone)]
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

#[cfg(test)]
mod tests {
    use crate::event_sink::order_index::{InMemoryKvIndex, KvIndex};
    use spectrum_offchain::clock::Clock;
    use std::ops::Add;
    use std::time::{Duration, Instant};

    struct AdjClock(Instant);

    impl Clock for AdjClock {
        fn current_time(&self) -> Instant {
            self.0
        }
    }

    #[test]
    fn query_before_after_eviction() {
        let delay = Duration::from_secs(10);
        let mut index = InMemoryKvIndex::new(delay, AdjClock(Instant::now()));
        index.put(1, 1);
        index.put(2, 2);
        index.register_for_eviction(1);
        index.run_eviction();
        assert_eq!(index.get(&1), Some(1));
        index.clock.0 = index.clock.0.add(delay);
        index.run_eviction();
        assert_eq!(index.get(&1), None);
        assert_eq!(index.get(&2), Some(2));
    }
}
