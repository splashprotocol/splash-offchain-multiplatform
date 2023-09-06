use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// Partitioned resource `R`.
/// `K` - partitioning key;
/// `N` - number of partitions.
pub struct Partitioned<const N: usize, K, R> {
    inner: [R; N],
    pd: PhantomData<K>,
}

impl<const N: usize, K, R> Partitioned<N, K, R>
where
    K: Hash,
{
    pub fn new(partitions: [R; N]) -> Self {
        Self {
            inner: partitions,
            pd: PhantomData::default(),
        }
    }

    pub fn get(&self, key: K) -> &R {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let hash = hasher.finish();
        &self.inner[(hash % N as u64) as usize]
    }
}
