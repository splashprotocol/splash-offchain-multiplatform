pub trait CanonicalHash {
    type Hash;
    fn canonical_hash(&self) -> Self::Hash;
}

#[cfg(feature = "cml-chain")]
impl CanonicalHash for cml_chain::transaction::Transaction {
    type Hash = cml_chain::crypto::TransactionHash;
    fn canonical_hash(&self) -> Self::Hash {
        cml_chain::crypto::hash::hash_transaction(&self.body)
    }
}
