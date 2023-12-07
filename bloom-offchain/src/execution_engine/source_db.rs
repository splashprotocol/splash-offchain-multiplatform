use std::collections::HashMap;

use crate::execution_engine::SourceId;

pub trait SourceDB<Src> {
    /// Take resolved source by [SourceId].
    /// Source is always available.
    fn take_unsafe(&mut self, source_id: SourceId) -> Src;
    fn put(&mut self, source_id: SourceId, source: Src);
    fn remove(&mut self, source_id: SourceId);
}

pub struct InMemorySourceDB<Src> {
    store: HashMap<SourceId, Src>,
}

impl<Src> SourceDB<Src> for InMemorySourceDB<Src> {
    fn take_unsafe(&mut self, source_id: SourceId) -> Src {
        self.store
            .remove(&source_id)
            .expect("Source is supposed to always be available")
    }

    fn put(&mut self, source_id: SourceId, source: Src) {
        self.store.insert(source_id, source);
    }

    fn remove(&mut self, source_id: SourceId) {
        self.store.remove(&source_id);
    }
}
