use spectrum_offchain::data::LiquiditySource;

pub trait StateIndexCache<Src: LiquiditySource> {
    fn insert(&mut self, src: Src) -> Option<Src>;
}