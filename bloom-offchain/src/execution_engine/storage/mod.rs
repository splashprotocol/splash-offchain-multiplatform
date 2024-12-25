use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};

use log::trace;

use spectrum_offchain::domain::event::{Confirmed, Predicted, Unconfirmed};
use spectrum_offchain::domain::{EntitySnapshot, Stable};

pub trait StateIndex<T: EntitySnapshot> {
    /// Get last confirmed state of the given entity.
    fn get_last_confirmed(&self, id: T::StableId) -> Option<Confirmed<T>>;
    /// Get last unconfirmed state of the given entity.
    fn get_last_unconfirmed(&self, id: T::StableId) -> Option<Unconfirmed<T>>;
    /// Get last predicted state of the given entity.
    fn get_last_predicted(&self, id: T::StableId) -> Option<Predicted<T>>;
    /// Get fallback state of the given entity.
    fn get_fallback(&self, id: T::StableId) -> Option<T>;
    /// Persist confirmed state of the entity.
    fn put_confirmed(&mut self, entity: Confirmed<T>);
    /// Persist unconfirmed state of the entity.
    fn put_unconfirmed(&mut self, entity: Unconfirmed<T>);
    /// Persist predicted state of the entity.
    fn put_predicted(&mut self, entity: Predicted<T>);
    fn put_fallback(&mut self, entity: T);
    fn invalidate_version(&mut self, ver: T::Version) -> Option<T::StableId>;
    fn eliminate<'a>(&mut self, sid: T::StableId);
    fn exists<'a>(&self, sid: &T::Version) -> bool;
    fn get_state<'a>(&self, sid: T::Version) -> Option<T>;
}

#[derive(Clone, Debug)]
pub struct StateIndexWithTracing<In>(pub In);

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

impl<In, T> StateIndex<T> for StateIndexWithTracing<In>
where
    In: StateIndex<T>,
    T: EntitySnapshot,
{
    fn get_last_confirmed(&self, id: T::StableId) -> Option<Confirmed<T>> {
        let res = self.0.get_last_confirmed(id);
        trace!("state_index::get_last_confirmed({}) -> {}", id, Displayed(&res));
        res
    }

    fn get_last_unconfirmed(&self, id: T::StableId) -> Option<Unconfirmed<T>> {
        let res = self.0.get_last_unconfirmed(id);
        trace!("state_index::get_last_unconfirmed({}) -> {}", id, Displayed(&res));
        res
    }

    fn get_last_predicted(&self, id: T::StableId) -> Option<Predicted<T>> {
        let res = self.0.get_last_predicted(id);
        trace!("state_index::get_last_predicted({}) -> {}", id, Displayed(&res));
        res
    }

    fn get_fallback(&self, id: T::StableId) -> Option<T> {
        let res = self.0.get_fallback(id);
        trace!("state_index::get_fallback({}) -> {}", id, Displayed(&res));
        res
    }

    fn put_confirmed(&mut self, entity: Confirmed<T>) {
        trace!(
            "state_index::put_confirmed(Entity({}, {}))",
            entity.0.stable_id(),
            entity.0.version()
        );
        self.0.put_confirmed(entity);
    }

    fn put_unconfirmed(&mut self, entity: Unconfirmed<T>) {
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

    fn put_fallback(&mut self, entity: T) {
        trace!(
            "state_index::put_fallback(Entity({}, {}))",
            entity.stable_id(),
            entity.version()
        );
        self.0.put_fallback(entity);
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

    fn eliminate<'a>(&mut self, id: T::StableId) {
        self.0.eliminate(id);
        trace!("state_index::eliminate({})", id);
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

#[derive(Clone)]
pub struct InMemoryStateIndex<T: EntitySnapshot> {
    store: HashMap<T::Version, T>,
    index: HashMap<InMemoryIndexKey, T::Version>,
}

impl<T: EntitySnapshot + Display> Debug for InMemoryStateIndex<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("InMemoryStateIndex(store:[")?;
        for (k, v) in &self.store {
            f.write_fmt(format_args!("{} -> {}, ", k, v))?;
        }
        f.write_str("], index:[")?;
        for (k, v) in &self.index {
            f.write_fmt(format_args!("{} -> {}, ", hex::encode(k), v))?;
        }
        f.write_str("])")
    }
}

impl<T: EntitySnapshot> InMemoryStateIndex<T> {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
            index: HashMap::new(),
        }
    }

    pub fn with_tracing() -> StateIndexWithTracing<Self> {
        StateIndexWithTracing(Self::new())
    }

    fn put(&mut self, prefix: u8, sid: T::StableId, value: T)
    where
        T::StableId: Into<[u8; 60]>,
    {
        let mut bound_versions = vec![];
        for p in ALL_STATE_KEYS {
            if p != prefix {
                let index_key = index_key(p, sid);
                if let Some(bound_ver) = self.index.get(&index_key) {
                    bound_versions.push(*bound_ver);
                }
            }
        }
        let index_key = index_key(prefix, sid);
        if let Some(old_ver) = self.index.get(&index_key) {
            // We remove version from store only in case
            // it is not bound to some other index.
            if !bound_versions.contains(old_ver) {
                self.store.remove(old_ver);
            }
        }
        let new_ver = value.version();
        self.index.insert(index_key, new_ver);
        self.store.insert(new_ver, value);
    }
}

type InMemoryIndexKey = [u8; 61];

const LAST_CONFIRMED_PREFIX: u8 = 3u8;
const LAST_UNCONFIRMED_PREFIX: u8 = 4u8;
const LAST_PREDICTED_PREFIX: u8 = 5u8;
const FALLBACK_PREFIX: u8 = 6u8;

const ALL_STATE_KEYS: [u8; 4] = [
    LAST_CONFIRMED_PREFIX,
    LAST_UNCONFIRMED_PREFIX,
    LAST_PREDICTED_PREFIX,
    FALLBACK_PREFIX,
];
const EPHEMERAL_STATE_KEYS: [u8; 3] = [LAST_UNCONFIRMED_PREFIX, LAST_PREDICTED_PREFIX, FALLBACK_PREFIX];

impl<T> StateIndex<T> for InMemoryStateIndex<T>
where
    T: EntitySnapshot + Clone,
    <T as EntitySnapshot>::Version: Copy + Debug + Eq,
    <T as Stable>::StableId: Copy + Into<[u8; 60]>,
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

    fn get_last_predicted(&self, id: T::StableId) -> Option<Predicted<T>> {
        let index_key = index_key(LAST_PREDICTED_PREFIX, id);
        self.index
            .get(&index_key)
            .and_then(|sid| self.store.get(sid))
            .map(|e| Predicted(e.clone()))
    }

    fn get_fallback(&self, id: T::StableId) -> Option<T> {
        let index_key = index_key(FALLBACK_PREFIX, id);
        self.index
            .get(&index_key)
            .and_then(|sid| self.store.get(sid))
            .map(|e| e.clone())
    }

    fn put_confirmed(&mut self, Confirmed(entity): Confirmed<T>) {
        let sid = entity.stable_id();
        self.put(LAST_CONFIRMED_PREFIX, sid, entity);
    }

    fn put_unconfirmed(&mut self, Unconfirmed(entity): Unconfirmed<T>) {
        let sid = entity.stable_id();
        self.put(LAST_UNCONFIRMED_PREFIX, sid, entity);
    }

    fn put_predicted(&mut self, Predicted(entity): Predicted<T>) {
        let sid = entity.stable_id();
        self.put(LAST_PREDICTED_PREFIX, sid, entity);
    }

    fn put_fallback(&mut self, entity: T) {
        let sid = entity.stable_id();
        self.put(FALLBACK_PREFIX, sid, entity);
    }

    fn invalidate_version(&mut self, ver: T::Version) -> Option<T::StableId> {
        if let Some(entity) = self.store.remove(&ver) {
            let sid = entity.stable_id();
            if let Entry::Occupied(index_ver) = self.index.entry(index_key(LAST_CONFIRMED_PREFIX, sid)) {
                if *index_ver.get() == ver {
                    if entity.is_quasi_permanent() {
                        // Confirmed state of quasi permanent entities cannot be dropped.
                        self.store.insert(ver, entity);
                    } else {
                        index_ver.remove();
                    }
                }
            }
            for prefix in EPHEMERAL_STATE_KEYS {
                if let Entry::Occupied(index_ver) = self.index.entry(index_key(prefix, sid)) {
                    if *index_ver.get() == ver {
                        index_ver.remove();
                    }
                }
            }
            return Some(sid);
        }
        None
    }

    fn eliminate(&mut self, sid: T::StableId) {
        for key in ALL_STATE_KEYS {
            if let Some(ver) = self.index.remove(&index_key(key, sid)) {
                self.store.remove(&ver);
            }
        }
    }

    fn exists(&self, sid: &T::Version) -> bool {
        self.store.contains_key(sid)
    }

    fn get_state(&self, sid: T::Version) -> Option<T> {
        self.store.get(&sid).map(|e| e.clone())
    }
}

fn index_key<T: Into<[u8; 60]>>(prefix: u8, id: T) -> InMemoryIndexKey {
    let mut arr = [prefix; 61];
    let raw_id: [u8; 60] = id.into();
    for (ix, byte) in raw_id.into_iter().enumerate() {
        arr[ix + 1] = byte;
    }
    arr
}

#[cfg(test)]
mod tests {
    use crate::execution_engine::storage::{InMemoryStateIndex, StateIndex, StateIndexWithTracing};
    use spectrum_cardano_lib::{OutputRef, Token};
    use spectrum_offchain::domain::event::{Confirmed, Predicted, Unconfirmed};
    use spectrum_offchain::domain::{EntitySnapshot, Stable};
    use std::fmt::{Display, Formatter};

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    struct Ent {
        stable_id: Token,
        ver: OutputRef,
    }

    impl Display for Ent {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.write_str(format!("Ent({})", &self.ver).as_str())
        }
    }

    impl Stable for Ent {
        type StableId = Token;
        fn stable_id(&self) -> Self::StableId {
            self.stable_id
        }
        fn is_quasi_permanent(&self) -> bool {
            true
        }
    }

    impl EntitySnapshot for Ent {
        type Version = OutputRef;
        fn version(&self) -> Self::Version {
            self.ver
        }
    }

    #[test]
    fn normal_index_cycle() {
        let stable_id = Token::from_string_unsafe("42019269344f20974cc563179e392a78dd3a3e9fe90adf30322abf8d.1af7822454e4e3286b8c59c3adbed84a7e4aa9467ae9741807d24de501ed48c2");
        let ver1 = OutputRef::from_string_unsafe(
            "9bdfa9a985ed742d70fe896868c50e97be3c8759b90d3c7f979e5becb75f8d86#1",
        );
        let ver2 = OutputRef::from_string_unsafe(
            "1af7822454e4e3286b8c59c3adbed84a7e4aa9467ae9741807d24de501ed48c2#0",
        );
        let mut store = StateIndexWithTracing(InMemoryStateIndex::<Ent>::new());
        store.put_unconfirmed(Unconfirmed(Ent { stable_id, ver: ver1 }));
        store.put_confirmed(Confirmed(Ent { stable_id, ver: ver1 }));
        store.put_predicted(Predicted(Ent { stable_id, ver: ver2 }));
        store.put_unconfirmed(Unconfirmed(Ent { stable_id, ver: ver2 }));
        let conf0 = store.get_last_confirmed(stable_id).unwrap().0;
        assert_eq!(conf0.ver, ver1);
        let pred1 = store.get_last_predicted(stable_id).unwrap().0;
        assert_eq!(pred1.ver, ver2);
        let unconf1 = store.get_last_unconfirmed(stable_id).unwrap().0;
        assert_eq!(unconf1.ver, ver2);
        store.invalidate_version(ver2);
        let conf1 = store.get_last_confirmed(stable_id).unwrap().0;
        assert_eq!(conf0.ver, ver1);
        let pred2 = store.get_last_predicted(stable_id);
        assert!(pred2.is_none());
        let unconf2 = store.get_last_unconfirmed(stable_id);
        assert!(unconf2.is_none());
    }
}
