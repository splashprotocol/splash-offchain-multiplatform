use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter, Write};

use log::trace;

use spectrum_offchain::data::event::{Confirmed, Predicted, Unconfirmed};
use spectrum_offchain::data::{EntitySnapshot, Stable};

pub mod kv_store;

pub trait StateIndex<T: EntitySnapshot> {
    /// Get last confirmed state of the given entity.
    fn get_last_confirmed<'a>(&self, id: T::StableId) -> Option<Confirmed<T>>;
    /// Get last unconfirmed state of the given entity.
    fn get_last_unconfirmed<'a>(&self, id: T::StableId) -> Option<Unconfirmed<T>>;
    /// Get last predicted state of the given entity.
    fn get_last_predicted<'a>(&self, id: T::StableId) -> Option<Predicted<T>>;
    /// Persist confirmed state of the entity.
    fn put_confirmed(&mut self, entity: Confirmed<T>);
    /// Persist unconfirmed state of the entity.
    fn put_unconfirmed(&mut self, entity: Unconfirmed<T>);
    /// Persist predicted state of the entity.
    fn put_predicted(&mut self, entity: Predicted<T>);
    fn invalidate_version(&mut self, ver: T::Version) -> Option<T::StableId>;
    fn eliminate<'a>(&mut self, sid: T::StableId);
    fn exists<'a>(&self, sid: &T::Version) -> bool;
    fn get_state<'a>(&self, sid: T::Version) -> Option<T>;
}

#[derive(Clone)]
pub struct StateIndexTracing<In>(pub In);

struct Displayed<'a, T>(&'a Option<T>);

impl<'a, T: EntitySnapshot> Display for Displayed<'a, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let repr = if let Some(t) = self.0 {
            format!("Some({})", t.version())
        } else {
            "None".to_string()
        };
        f.write_str(repr.as_str())
    }
}

impl<In, T> StateIndex<T> for StateIndexTracing<In>
where
    In: StateIndex<T>,
    T: EntitySnapshot,
{
    fn get_last_confirmed<'a>(&self, id: T::StableId) -> Option<Confirmed<T>> {
        let res = self.0.get_last_confirmed(id);
        trace!("state_index::get_last_confirmed({}) -> {}", id, Displayed(&res));
        res
    }

    fn get_last_unconfirmed<'a>(&self, id: T::StableId) -> Option<Unconfirmed<T>> {
        let res = self.0.get_last_unconfirmed(id);
        trace!("state_index::get_last_unconfirmed({}) -> {}", id, Displayed(&res));
        res
    }

    fn get_last_predicted<'a>(&self, id: T::StableId) -> Option<Predicted<T>> {
        let res = self.0.get_last_predicted(id);
        trace!("state_index::get_last_predicted({}) -> {}", id, Displayed(&res));
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

    fn put_predicted(&mut self, entity: Predicted<T>) {
        trace!(
            "state_index::put_predicted(Entity({}, {}))",
            entity.0.stable_id(),
            entity.0.version()
        );
        self.0.put_predicted(entity);
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

    fn exists<'a>(&self, sid: &T::Version) -> bool {
        let res = self.0.exists(sid);
        trace!("state_index::may_exist({}) -> {}", sid, res);
        res
    }

    fn get_state<'a>(&self, sid: T::Version) -> Option<T> {
        let res = self.0.get_state(sid);
        trace!("state_index::get_state({}) -> {}", sid, Displayed(&res));
        res
    }
}

const MAX_ROLLBACK_DEPTH: usize = 32;

#[derive(Clone)]
pub struct InMemoryStateIndex<T: EntitySnapshot> {
    store: HashMap<T::Version, T>,
    index: HashMap<InMemoryIndexKey, T::Version>,
}

impl<T: EntitySnapshot> InMemoryStateIndex<T> {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
            index: HashMap::new(),
        }
    }

    fn put(&mut self, index_key: InMemoryIndexKey, value: T) {
        if let Some(old_ver) = self.index.get(&index_key) {
            self.store.remove(old_ver);
        }
        let new_ver = value.version();
        self.index.insert(index_key, new_ver);
        self.store.insert(new_ver, value);
    }
}

type InMemoryIndexKey = [u8; 29];

const LAST_CONFIRMED_PREFIX: u8 = 3u8;
const LAST_UNCONFIRMED_PREFIX: u8 = 4u8;
const LAST_PREDICTED_PREFIX: u8 = 5u8;

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

    fn get_last_predicted<'a>(&self, id: T::StableId) -> Option<Predicted<T>> {
        let index_key = index_key(LAST_PREDICTED_PREFIX, id);
        self.index
            .get(&index_key)
            .and_then(|sid| self.store.get(sid))
            .map(|e| Predicted(e.clone()))
    }

    fn put_confirmed(&mut self, Confirmed(entity): Confirmed<T>) {
        let sid = entity.stable_id();
        let index_key = index_key(LAST_CONFIRMED_PREFIX, sid);
        self.put(index_key, entity);
    }

    fn put_unconfirmed(&mut self, Unconfirmed(entity): Unconfirmed<T>) {
        let sid = entity.stable_id();
        let index_key = index_key(LAST_UNCONFIRMED_PREFIX, sid);
        self.put(index_key, entity);
    }

    fn put_predicted(&mut self, Predicted(entity): Predicted<T>) {
        let sid = entity.stable_id();
        let index_key = index_key(LAST_PREDICTED_PREFIX, sid);
        self.put(index_key, entity);
    }

    fn invalidate_version(&mut self, ver: T::Version) -> Option<T::StableId> {
        if let Some(entity) = self.store.get(&ver) {
            let sid = entity.stable_id();
            self.index.remove(&index_key(LAST_PREDICTED_PREFIX, sid));
            self.index.remove(&index_key(LAST_UNCONFIRMED_PREFIX, sid));
            self.index.remove(&index_key(LAST_CONFIRMED_PREFIX, sid));
            self.store.remove(&ver);
            return Some(sid);
        }
        None
    }

    fn eliminate(&mut self, sid: T::StableId) {
        let predicted_ver = self.index.remove(&index_key(LAST_PREDICTED_PREFIX, sid));
        let unconfirmed_ver = self.index.remove(&index_key(LAST_UNCONFIRMED_PREFIX, sid));
        let confirmed_ver = self.index.remove(&index_key(LAST_PREDICTED_PREFIX, sid));
        if let Some(ver) = predicted_ver {
            self.store.remove(&ver);
        }
        if let Some(ver) = unconfirmed_ver {
            self.store.remove(&ver);
        }
        if let Some(ver) = confirmed_ver {
            self.store.remove(&ver);
        }
    }

    fn exists(&self, sid: &T::Version) -> bool {
        self.store.contains_key(sid)
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
