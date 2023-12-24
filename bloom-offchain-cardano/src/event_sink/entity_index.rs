use spectrum_offchain::data::EntitySnapshot;

pub trait EntityIndex<T: EntitySnapshot> {
    fn put_state(&mut self, state: T);
    fn take_state(&mut self, ver: T::Version) -> Option<T>;
    fn exists(&self, ver: T::Version) -> bool;
}
