use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Display};
use std::time::{Duration, SystemTime};

use log::trace;

use spectrum_offchain::data::order::SpecializedOrder;

pub trait OrderIndex<T: SpecializedOrder> {
    fn put(&mut self, state: T);
    fn get(&mut self, id: &T::TOrderId) -> Option<T>;
    fn exists(&self, id: &T::TOrderId) -> bool;
    /// Mark an entry identified by the given [T::TOrderId] as subject for future eviction
    fn register_for_eviction(&mut self, id: T::TOrderId);
    /// Evict outdated entries.
    fn run_eviction(&mut self);
}

#[derive(Clone)]
pub struct InMemoryOrderIndex<T: SpecializedOrder> {
    store: HashMap<T::TOrderId, T>,
    eviction_queue: VecDeque<(SystemTime, T::TOrderId)>,
    eviction_delay: Duration,
}

impl<T: SpecializedOrder> InMemoryOrderIndex<T> {
    pub fn new(eviction_delay: Duration) -> Self {
        Self {
            store: Default::default(),
            eviction_queue: Default::default(),
            eviction_delay,
        }
    }
    pub fn with_tracing(self) -> OrderIndexTracing<Self> {
        OrderIndexTracing::attach(self)
    }
}

impl<T> OrderIndex<T> for InMemoryOrderIndex<T>
where
    T: SpecializedOrder + Clone,
    T::TOrderId: Display,
{
    fn put(&mut self, state: T) {
        self.store.insert(state.get_self_ref(), state);
    }

    fn get(&mut self, id: &T::TOrderId) -> Option<T> {
        self.store.get(&id).cloned()
    }

    fn exists(&self, id: &T::TOrderId) -> bool {
        self.store.contains_key(&id)
    }

    fn register_for_eviction(&mut self, id: T::TOrderId) {
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
}

impl<R> OrderIndexTracing<R> {
    pub fn attach(repo: R) -> Self {
        Self { inner: repo }
    }
}

impl<T, R> OrderIndex<T> for OrderIndexTracing<R>
where
    T: SpecializedOrder + Debug,
    T::TOrderId: Display,
    R: OrderIndex<T>,
{
    fn put(&mut self, state: T) {
        trace!(target: "offchain", "OrderIndex::put_state({:?})", state);
        self.inner.put(state)
    }

    fn get(&mut self, id: &T::TOrderId) -> Option<T> {
        trace!(target: "offchain", "OrderIndex::get_state({})", id);
        self.inner.get(id)
    }

    fn exists(&self, id: &T::TOrderId) -> bool {
        let res = self.inner.exists(id);
        trace!(target: "offchain", "OrderIndex::exists({}) -> {}", id, res);
        res
    }

    fn register_for_eviction(&mut self, id: T::TOrderId) {
        trace!(target: "offchain", "OrderIndex::register_for_eviction({})", id);
        self.inner.register_for_eviction(id)
    }

    fn run_eviction(&mut self) {
        trace!(target: "offchain", "OrderIndex::run_eviction()");
        self.inner.run_eviction()
    }
}
