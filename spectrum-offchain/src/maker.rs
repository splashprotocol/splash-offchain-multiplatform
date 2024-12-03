/// Instantiate [Self] given context [T].
pub trait Maker<Key, T> {
    fn make(key: Key, ctx: &T) -> Self;
}
