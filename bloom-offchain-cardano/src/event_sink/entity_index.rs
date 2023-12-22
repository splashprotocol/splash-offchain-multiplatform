use spectrum_offchain::data::LiquiditySource;

pub trait EntityIndex<T: LiquiditySource> {
    fn put_state(&mut self, state: T);
    fn take_state(&mut self, ver: T::Version) -> Option<T>;
    fn exists(&self, ver: T::Version) -> bool;
}
