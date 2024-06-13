pub trait CannonicalHash {
    type Hash;
    fn canonical_hash(&self) -> Self::Hash;
}
