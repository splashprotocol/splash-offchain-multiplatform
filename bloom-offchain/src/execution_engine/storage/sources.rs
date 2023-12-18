use std::collections::HashMap;
use std::hash::Hash;

pub trait Sources<Key, Src> {
    /// Take resolved source by [Key].
    /// Source is always available.
    fn take_unsafe(&mut self, key: Key) -> Src;
    fn put(&mut self, key: Key, source: Src);
    fn remove(&mut self, key: Key);
}

pub struct InMemorySourceDB<Key, Src> {
    store: HashMap<Key, Src>,
}

impl<Key, Src> Sources<Key, Src> for InMemorySourceDB<Key, Src>
where
    Key: Eq + Copy + Hash,
{
    fn take_unsafe(&mut self, key: Key) -> Src {
        self.store
            .remove(&key)
            .expect("Source is supposed to always be available")
    }

    fn put(&mut self, key: Key, source: Src) {
        self.store.insert(key, source);
    }

    fn remove(&mut self, key: Key) {
        self.store.remove(&key);
    }
}
