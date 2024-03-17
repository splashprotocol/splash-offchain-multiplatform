/// Instantiate [Self] given context [T].
pub trait Maker<T> {
    fn make(ctx: &T) -> Self;
}
