use std::collections::HashMap;
use std::hash::Hash;

use spectrum_offchain::data::EntitySnapshot;

pub trait StateIndexCache<StableId, Src: EntitySnapshot<StableId = StableId>> {
    fn insert(&mut self, src: Src) -> Option<Src>;
    fn get(&self, id: StableId) -> Option<Src>;
}

#[derive(Debug, Clone)]
pub struct InMemoryStateIndexCache<StableId, Src>(HashMap<StableId, Src>);

impl<StableId, Src> InMemoryStateIndexCache<StableId, Src> {
    pub fn new() -> Self {
        Self(Default::default())
    }
}

impl<StableId, Src> StateIndexCache<StableId, Src> for InMemoryStateIndexCache<StableId, Src>
where
    StableId: Eq + Hash,
    Src: Clone + EntitySnapshot<StableId = StableId>,
{
    fn insert(&mut self, src: Src) -> Option<Src> {
        self.0.insert(src.stable_id(), src)
    }

    fn get(&self, id: StableId) -> Option<Src> {
        self.0.get(&id).cloned()
    }
}
