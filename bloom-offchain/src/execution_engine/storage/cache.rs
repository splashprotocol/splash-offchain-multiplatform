use spectrum_offchain::data::EntitySnapshot;

pub trait StateIndexCache<StableId, Src: EntitySnapshot<StableId = StableId>> {
    fn insert(&mut self, src: Src) -> Option<Src>;
    fn get(&self, id: StableId) -> Option<Src>;
}
