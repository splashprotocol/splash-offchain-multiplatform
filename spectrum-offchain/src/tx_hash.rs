pub trait CanonicalHash {
    type Hash;
    fn canonical_hash(&self) -> Self::Hash;
}
