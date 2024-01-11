use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Display};
use std::time::{Duration, SystemTime};

use log::trace;

use spectrum_offchain::data::EntitySnapshot;

pub trait EntityIndex<T: EntitySnapshot> {
    fn put_state(&mut self, state: T);
    fn get_state(&mut self, ver: &T::Version) -> Option<T>;
    fn remove_state(&mut self, ver: &T::Version);
    fn exists(&self, ver: &T::Version) -> bool;
    /// Mark an entry identified by the given [T::Version] as subject for future eviction
    fn register_for_eviction(&mut self, ver: T::Version);
    /// Evict outdated entries.
    fn run_eviction(&mut self);
}

#[derive(Clone)]
pub struct InMemoryEntityIndex<T: EntitySnapshot> {
    store: HashMap<T::Version, T>,
    eviction_queue: VecDeque<(SystemTime, T::Version)>,
    eviction_delay: Duration,
}

impl<T: EntitySnapshot> InMemoryEntityIndex<T> {
    pub fn new(eviction_delay: Duration) -> Self {
        Self {
            store: Default::default(),
            eviction_queue: Default::default(),
            eviction_delay,
        }
    }
    pub fn with_tracing(self) -> EntityIndexTracing<Self> {
        EntityIndexTracing::attach(self)
    }
}

impl<T> EntityIndex<T> for InMemoryEntityIndex<T>
where
    T: EntitySnapshot + Clone,
{
    fn put_state(&mut self, state: T) {
        self.store.insert(state.version(), state);
    }

    fn get_state(&mut self, ver: &T::Version) -> Option<T> {
        self.store.get(&ver).cloned()
    }

    fn remove_state(&mut self, ver: &T::Version) {
        self.store.remove(ver);
    }

    fn exists(&self, ver: &T::Version) -> bool {
        self.store.contains_key(&ver)
    }

    fn register_for_eviction(&mut self, ver: T::Version) {
        let now = SystemTime::now();
        self.eviction_queue.push_back((now + self.eviction_delay, ver));
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

pub struct EntityIndexTracing<R> {
    inner: R,
}

impl<R> EntityIndexTracing<R> {
    pub fn attach(repo: R) -> Self {
        Self { inner: repo }
    }
}

impl<T, R> EntityIndex<T> for EntityIndexTracing<R>
where
    T: EntitySnapshot + Debug,
    T::Version: Display,
    R: EntityIndex<T>,
{
    fn put_state(&mut self, state: T) {
        trace!("put_state({:?})", state);
        self.inner.put_state(state)
    }

    fn get_state(&mut self, ver: &T::Version) -> Option<T> {
        trace!("get_state({})", ver);
        self.inner.get_state(ver)
    }

    fn remove_state(&mut self, ver: &T::Version) {
        trace!("remove_state({})", ver);
        self.inner.remove_state(ver)
    }

    fn exists(&self, ver: &T::Version) -> bool {
        let res = self.inner.exists(ver);
        trace!("exists({}) -> {}", ver, res);
        res
    }

    fn register_for_eviction(&mut self, ver: T::Version) {
        trace!("register_for_eviction({})", ver);
        self.inner.register_for_eviction(ver)
    }

    fn run_eviction(&mut self) {
        trace!("run_eviction()");
        self.inner.run_eviction()
    }
}
