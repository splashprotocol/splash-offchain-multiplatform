use std::collections::HashMap;

use spectrum_offchain::data::EntitySnapshot;

pub trait EntityIndex<T: EntitySnapshot> {
    fn put_state(&mut self, state: T);
    fn take_state(&mut self, ver: &T::Version) -> Option<T>;
    fn exists(&self, ver: &T::Version) -> bool;
}

#[derive(Clone)]
pub struct InMemoryEntityIndex<T: EntitySnapshot>(HashMap<T::Version, T>);

impl<T: EntitySnapshot> InMemoryEntityIndex<T> {
    pub fn new() -> Self {
        Self(Default::default())
    }
}

impl<T: EntitySnapshot> EntityIndex<T> for InMemoryEntityIndex<T> {
    fn put_state(&mut self, state: T) {
        self.0.insert(state.version(), state);
    }

    fn take_state(&mut self, ver: &T::Version) -> Option<T> {
        self.0.remove(&ver)
    }

    fn exists(&self, ver: &T::Version) -> bool {
        self.0.contains_key(&ver)
    }
}
