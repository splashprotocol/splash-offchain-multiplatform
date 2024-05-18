use circular_buffer::CircularBuffer;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt::Debug;

use log::trace;

use spectrum_offchain::data::{EntitySnapshot, Stable};

pub mod kv_store;

pub trait StateIndex<T: EntitySnapshot> {
    /// Get last confirmed state of the given entity.
    fn get<'a>(&self, id: &T::StableId) -> Option<T>;
    /// Persist confirmed state of the entity.
    fn put<'a>(&mut self, entity: T) -> bool;
    /// Invalidate particular state of the entity.
    fn invalidate<'a>(&mut self, ver: T::Version) -> Option<T::StableId>;
    /// Invalidate particular state of the entity.
    fn eliminate<'a>(&mut self, ver: T::Version);
    /// False-positive analog of `exists()`.
    fn may_exist<'a>(&self, sid: T::Version) -> bool;
    fn get_state<'a>(&self, sid: T::Version) -> Option<T>;
}

#[derive(Clone)]
pub struct StateIndexTracing<In>(pub In);

impl<In, Src> StateIndex<Src> for StateIndexTracing<In>
where
    In: StateIndex<Src>,
    Src: EntitySnapshot,
{
    fn get<'a>(&self, id: &Src::StableId) -> Option<Src> {
        let res = self.0.get(id);
        trace!(
            "state_index::get_last_confirmed({}) -> {}",
            id,
            if res.is_some() { "Some(_)" } else { "None" }
        );
        res
    }

    fn put<'a>(&mut self, entity: Src) -> bool {
        let sid = entity.stable_id();
        let ver = entity.version();
        let res = self.0.put(entity);
        trace!("state_index::put(Entity({}, {})) -> {}", sid, ver, res);
        res
    }

    fn invalidate<'a>(&mut self, ver: Src::Version) -> Option<Src::StableId> {
        let res = self.0.invalidate(ver);
        trace!(
            "state_index::invalidate({}) -> {}",
            ver,
            if res.is_some() { "Some(_)" } else { "None" }
        );
        res
    }

    fn eliminate<'a>(&mut self, ver: Src::Version) {
        self.0.eliminate(ver);
        trace!("state_index::eliminate({})", ver);
    }

    fn may_exist<'a>(&self, sid: Src::Version) -> bool {
        let res = self.0.may_exist(sid);
        trace!("state_index::may_exist({}) -> {}", sid, res);
        res
    }

    fn get_state<'a>(&self, sid: Src::Version) -> Option<Src> {
        let res = self.0.get_state(sid);
        trace!(
            "state_index::get_state({}) -> {}",
            sid,
            if res.is_some() { "Some(_)" } else { "None" }
        );
        res
    }
}

const MAX_ROLLBACK_DEPTH: usize = 32;

#[derive(Debug, Clone)]
pub struct InMemoryStateIndex<T: EntitySnapshot> {
    store: HashMap<T::Version, T>,
    index: HashMap<T::StableId, CircularBuffer<MAX_ROLLBACK_DEPTH, T::Version>>,
}

impl<T: EntitySnapshot> InMemoryStateIndex<T> {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
            index: HashMap::new(),
        }
    }
}

impl<T> StateIndex<T> for InMemoryStateIndex<T>
where
    T: EntitySnapshot + Clone,
    <T as EntitySnapshot>::Version: Copy + Debug,
    <T as Stable>::StableId: Copy,
{
    fn get(&self, id: &T::StableId) -> Option<T> {
        self.index
            .get(&id)
            .and_then(|versions| versions.back())
            .and_then(|version| self.store.get(version))
            .map(|e| e.clone())
    }

    fn put(&mut self, entity: T) -> bool {
        match self.index.entry(entity.stable_id()) {
            Entry::Occupied(mut entry) => entry.get_mut().push_back(entity.version()),
            Entry::Vacant(entry) => {
                entry.insert(CircularBuffer::from([entity.version()]));
            }
        }
        self.store.insert(entity.version(), entity).is_some()
    }

    fn invalidate<'a>(&mut self, ver: T::Version) -> Option<T::StableId> {
        if let Some(entity) = self.store.remove(&ver) {
            let id = entity.stable_id();
            if let Some(versions) = self.index.get_mut(&id) {
                loop {
                    match versions.pop_back() {
                        // Roll back all versions after `ver` (including).
                        Some(rolled_back_ver) if rolled_back_ver != ver => continue,
                        _ => break,
                    }
                }
            }
            return Some(id);
        }
        None
    }

    fn eliminate<'a>(&mut self, ver: T::Version) {
        if let Some(entity) = self.store.remove(&ver) {
            let id = entity.stable_id();
            self.index.remove(&id);
        }
    }

    fn may_exist(&self, sid: T::Version) -> bool {
        self.store.contains_key(&sid)
    }

    fn get_state(&self, sid: T::Version) -> Option<T> {
        self.store.get(&sid).map(|e| e.clone())
    }
}
