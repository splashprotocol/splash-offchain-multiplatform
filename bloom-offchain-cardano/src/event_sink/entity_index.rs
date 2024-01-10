use std::collections::{HashMap, VecDeque};
use std::time::SystemTime;

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
pub struct InMemoryEntityIndex<T: EntitySnapshot>(HashMap<T::Version, T>, VecDeque<(SystemTime, T::Version)>);

impl<T: EntitySnapshot> InMemoryEntityIndex<T> {
    pub fn new() -> Self {
        Self(Default::default(), Default::default())
    }
}

impl<T: EntitySnapshot> EntityIndex<T> for InMemoryEntityIndex<T> {
    fn put_state(&mut self, state: T) {
        self.0.insert(state.version(), state);
    }

    fn get_state(&mut self, ver: &T::Version) -> Option<T> {
        self.0.remove(&ver)
    }

    fn remove_state(&mut self, ver: &T::Version) {
        self.0.remove(ver);
    }

    fn exists(&self, ver: &T::Version) -> bool {
        self.0.contains_key(&ver)
    }

    fn register_for_eviction(&mut self, ver: T::Version) {
        let now = SystemTime::now();
        self.1.push_back((now, ver));
    }

    fn run_eviction(&mut self) {
        let now = SystemTime::now();
        loop {
            match self.1.pop_front() {
                Some((ts, v)) if ts <= now => {
                    self.0.remove(&v);
                    continue;
                }
                Some((ts, v)) => self.1.push_front((ts, v)),
                _ => {}
            }
            break;
        }
    }
}
