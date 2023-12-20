use spectrum_offchain::data::LiquiditySource;

pub trait StateIndexCache<StableId, Src: LiquiditySource<StableId = StableId>> {
    fn insert(&mut self, src: Src) -> Option<Src>;
    fn get(&self, id: StableId) -> Option<Src>;
}
