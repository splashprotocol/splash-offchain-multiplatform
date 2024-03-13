use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Display};
use std::time::{Duration, SystemTime};

use log::trace;

use spectrum_offchain::data::{EntitySnapshot, Tradable};

pub trait TradableEntityIndex<T: EntitySnapshot + Tradable> {
    fn put_state(&mut self, state: T);
    fn get_state(&mut self, ver: &T::Version) -> Option<T>;
    fn pair_of(&self, id: &T::StableId) -> Option<T::PairId>;
    fn exists(&self, ver: &T::Version) -> bool;
    /// Mark an entry identified by the given [T::Version] as subject for future eviction
    fn register_for_eviction(&mut self, ver: T::Version);
    /// Evict outdated entries.
    fn run_eviction(&mut self);
}

#[derive(Clone)]
pub struct InMemoryEntityIndex<T: EntitySnapshot + Tradable> {
    store: HashMap<T::Version, T>,
    permanent_pairs: HashMap<T::StableId, T::PairId>,
    eviction_queue: VecDeque<(SystemTime, T::Version)>,
    eviction_delay: Duration,
}

impl<T: EntitySnapshot + Tradable> InMemoryEntityIndex<T> {
    pub fn new(eviction_delay: Duration) -> Self {
        Self {
            store: Default::default(),
            permanent_pairs: Default::default(),
            eviction_queue: Default::default(),
            eviction_delay,
        }
    }
    pub fn with_tracing(self) -> EntityIndexTracing<Self> {
        EntityIndexTracing::attach(self)
    }
}

impl<T> TradableEntityIndex<T> for InMemoryEntityIndex<T>
where
    T: EntitySnapshot + Tradable + Clone,
{
    fn put_state(&mut self, state: T) {
        if state.is_quasi_permanent() {
            self.permanent_pairs.insert(state.stable_id(), state.pair_id());
        }
        self.store.insert(state.version(), state);
    }

    fn get_state(&mut self, ver: &T::Version) -> Option<T> {
        self.store.get(&ver).cloned()
    }

    fn pair_of(&self, id: &T::StableId) -> Option<T::PairId> {
        self.permanent_pairs.get(id).map(|pid| *pid)
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

impl<T, R> TradableEntityIndex<T> for EntityIndexTracing<R>
where
    T: EntitySnapshot + Tradable + Debug,
    T::Version: Display,
    R: TradableEntityIndex<T>,
{
    fn put_state(&mut self, state: T) {
        trace!(target: "offchain", "EntityIndex::put_state({:?})", state);
        self.inner.put_state(state)
    }

    fn get_state(&mut self, ver: &T::Version) -> Option<T> {
        trace!(target: "offchain", "EntityIndex::get_state({})", ver);
        self.inner.get_state(ver)
    }

    fn pair_of(&self, id: &T::StableId) -> Option<T::PairId> {
        trace!(target: "offchain", "EntityIndex::pair_of({})", id);
        self.inner.pair_of(id)
    }

    fn exists(&self, ver: &T::Version) -> bool {
        let res = self.inner.exists(ver);
        trace!(target: "offchain", "EntityIndex::exists({}) -> {}", ver, res);
        res
    }

    fn register_for_eviction(&mut self, ver: T::Version) {
        trace!(target: "offchain", "EntityIndex::register_for_eviction({})", ver);
        self.inner.register_for_eviction(ver)
    }

    fn run_eviction(&mut self) {
        trace!(target: "offchain", "EntityIndex::run_eviction()");
        self.inner.run_eviction()
    }
}
