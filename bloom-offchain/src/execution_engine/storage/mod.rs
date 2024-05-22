use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt::Debug;

use log::{trace};

use spectrum_offchain::circular_filter::CircularFilter;
use spectrum_offchain::data::unique_entity::{Confirmed, Unconfirmed};
use spectrum_offchain::data::{EntitySnapshot, Stable};

pub mod kv_store;

pub trait StateIndex<T: EntitySnapshot> {
    /// Get last confirmed state of the given entity.
    fn get_last_confirmed<'a>(&self, id: T::StableId) -> Option<Confirmed<T>>;
    /// Get last unconfirmed state of the given entity.
    fn get_last_unconfirmed<'a>(&self, id: T::StableId) -> Option<Unconfirmed<T>>;
    /// Persist confirmed state of the entity.
    fn put_confirmed(&mut self, entity: Confirmed<T>);
    /// Persist unconfirmed state of the entity.
    fn put_unconfirmed(&mut self, entity: Unconfirmed<T>);
    fn rollback_inclusive(&mut self, ver: T::Version);
    fn invalidate_version(&mut self, ver: T::Version) -> Option<T::StableId>;
    fn eliminate<'a>(&mut self, sid: T::StableId);
    /// False-positive analog of `exists()`.
    fn may_exist<'a>(&self, sid: T::Version) -> bool;
    fn get_state<'a>(&self, sid: T::Version) -> Option<T>;
}

#[derive(Clone)]
pub struct StateIndexTracing<In>(pub In);

impl<In, T> StateIndex<T> for StateIndexTracing<In>
where
    In: StateIndex<T>,
    T: EntitySnapshot,
{
    fn get_last_confirmed<'a>(&self, id: T::StableId) -> Option<Confirmed<T>> {
        let res = self.0.get_last_confirmed(id);
        trace!(
            "state_index::get_last_confirmed({}) -> {}",
            id,
            if res.is_some() { "Some(_)" } else { "None" }
        );
        res
    }

    fn get_last_unconfirmed<'a>(&self, id: T::StableId) -> Option<Unconfirmed<T>> {
        let res = self.0.get_last_unconfirmed(id);
        trace!(
            "state_index::get_last_unconfirmed({}) -> {}",
            id,
            if res.is_some() { "Some(_)" } else { "None" }
        );
        res
    }

    fn put_confirmed<'a>(&mut self, entity: Confirmed<T>) {
        trace!(
            "state_index::put_confirmed(Entity({}, {}))",
            entity.0.stable_id(),
            entity.0.version()
        );
        self.0.put_confirmed(entity);
    }

    fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<T>) {
        trace!(
            "state_index::put_unconfirmed(Entity({}, {}))",
            entity.0.stable_id(),
            entity.0.version()
        );
        self.0.put_unconfirmed(entity);
    }

    fn rollback_inclusive<'a>(&mut self, ver: T::Version) {
        self.0.rollback_inclusive(ver);
        trace!("state_index::rollback_inclusive({})", ver);
    }

    fn invalidate_version(&mut self, ver: T::Version) -> Option<T::StableId> {
        let res = self.0.invalidate_version(ver);
        trace!(
            "state_index::invalidate_version({}) -> {}",
            ver,
            if res.is_some() { "Some(_)" } else { "None" }
        );
        res
    }

    fn eliminate<'a>(&mut self, ver: T::StableId) {
        self.0.eliminate(ver);
        trace!("state_index::eliminate({})", ver);
    }

    fn may_exist<'a>(&self, sid: T::Version) -> bool {
        let res = self.0.may_exist(sid);
        trace!("state_index::may_exist({}) -> {}", sid, res);
        res
    }

    fn get_state<'a>(&self, sid: T::Version) -> Option<T> {
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

#[derive(Clone)]
pub struct InMemoryStateIndex<T: EntitySnapshot> {
    store: HashMap<T::Version, T>,
    index: HashMap<InMemoryIndexKey, T::Version>,
    versions: HashMap<T::StableId, CircularFilter<MAX_ROLLBACK_DEPTH, T::Version>>,
}

impl<T: EntitySnapshot> InMemoryStateIndex<T> {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
            versions: HashMap::new(),
            index: HashMap::new(),
        }
    }

    fn add_version<'a>(&mut self, sid: T::StableId, new_ver: T::Version)
    where
        T: EntitySnapshot + Clone,
        <T as EntitySnapshot>::Version: Copy + Debug,
        <T as Stable>::StableId: Copy + Into<[u8; 28]>,
    {
        match self.versions.entry(sid) {
            Entry::Occupied(mut entry) => {
                let versions = entry.get_mut();
                if !versions.contains(&new_ver) {
                    if let Some(elim_ver) = versions.add(new_ver) {
                        self.store.remove(&elim_ver);
                    }
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(CircularFilter::one(new_ver));
            }
        }
    }
}

type InMemoryIndexKey = [u8; 29];

const LAST_CONFIRMED_PREFIX: u8 = 3u8;
const LAST_UNCONFIRMED_PREFIX: u8 = 4u8;

impl<T> StateIndex<T> for InMemoryStateIndex<T>
where
    T: EntitySnapshot + Clone,
    <T as EntitySnapshot>::Version: Copy + Debug + Eq,
    <T as Stable>::StableId: Copy + Into<[u8; 28]>,
{
    fn get_last_confirmed(&self, id: T::StableId) -> Option<Confirmed<T>> {
        let index_key = index_key(LAST_CONFIRMED_PREFIX, id);
        self.index
            .get(&index_key)
            .and_then(|sid| self.store.get(sid))
            .map(|e| Confirmed(e.clone()))
    }

    fn get_last_unconfirmed(&self, id: T::StableId) -> Option<Unconfirmed<T>> {
        let index_key = index_key(LAST_UNCONFIRMED_PREFIX, id);
        self.index
            .get(&index_key)
            .and_then(|sid| self.store.get(sid))
            .map(|e| Unconfirmed(e.clone()))
    }

    fn put_confirmed(&mut self, Confirmed(entity): Confirmed<T>) {
        let sid = entity.stable_id();
        let index_key = index_key(LAST_CONFIRMED_PREFIX, sid);
        let new_ver = entity.version();
        self.index.insert(index_key, new_ver);
        self.add_version(sid, new_ver);
        self.store.insert(new_ver, entity);
    }

    fn put_unconfirmed(&mut self, Unconfirmed(entity): Unconfirmed<T>) {
        let sid = entity.stable_id();
        let index_key = index_key(LAST_UNCONFIRMED_PREFIX, sid);
        let new_ver = entity.version();
        self.index.insert(index_key, new_ver);
        self.add_version(sid, new_ver);
        self.store.insert(new_ver, entity);
    }

    fn rollback_inclusive(&mut self, inv_ver: T::Version) {
        if let Some(entity) = self.store.remove(&inv_ver) {
            let sid = entity.stable_id();
            let confirmed_index_key = index_key(LAST_CONFIRMED_PREFIX, sid);
            let unconfirmed_index_key = index_key(LAST_UNCONFIRMED_PREFIX, sid);
            let best_confirmed_version = self.index.get(&confirmed_index_key);
            let mut best_confirmed_version_crossed = false;
            if let Some(versions) = self.versions.get_mut(&sid) {
                loop {
                    match versions.pop_back() {
                        // Roll back all versions after `ver` (including).
                        Some(rolled_back_ver) if rolled_back_ver != inv_ver => {
                            if best_confirmed_version
                                .map(|v| *v == rolled_back_ver)
                                .unwrap_or(false)
                            {
                                best_confirmed_version_crossed = true;
                            }
                            trace!(
                                "Removing version {} while invalidating version {}",
                                rolled_back_ver,
                                inv_ver
                            );
                            self.store.remove(&rolled_back_ver);
                            continue;
                        }
                        _ => break,
                    }
                }
                if let Some(new_best_version) = versions.back() {
                    if best_confirmed_version_crossed {
                        self.index.remove(&unconfirmed_index_key);
                        self.index.insert(confirmed_index_key, *new_best_version);
                    } else {
                        self.index.insert(unconfirmed_index_key, *new_best_version);
                    }
                }
            }
        }
    }

    fn invalidate_version(&mut self, ver: T::Version) -> Option<T::StableId> {
        if let Some(entity) = self.store.get(&ver) {
            let sid = entity.stable_id();
            let confirmed_index_key = index_key(LAST_CONFIRMED_PREFIX, sid);
            let unconfirmed_index_key = index_key(LAST_UNCONFIRMED_PREFIX, sid);
            if self
                .index
                .get(&confirmed_index_key)
                .map(|v| *v == ver)
                .unwrap_or(false)
            {
                self.index.remove(&unconfirmed_index_key);
                self.index.remove(&confirmed_index_key);
            } else if self
                .index
                .get(&unconfirmed_index_key)
                .map(|v| *v == ver)
                .unwrap_or(false)
            {
                self.index.remove(&unconfirmed_index_key);
            }
            return Some(sid);
        }
        None
    }

    fn eliminate(&mut self, sid: T::StableId) {}

    fn may_exist(&self, sid: T::Version) -> bool {
        self.store.contains_key(&sid)
    }

    fn get_state(&self, sid: T::Version) -> Option<T> {
        self.store.get(&sid).map(|e| e.clone())
    }
}

pub fn index_key<T: Into<[u8; 28]>>(prefix: u8, id: T) -> InMemoryIndexKey {
    let mut arr = [prefix; 29];
    let raw_id: [u8; 28] = id.into();
    for (ix, byte) in raw_id.into_iter().enumerate() {
        arr[ix + 1] = byte;
    }
    arr
}
