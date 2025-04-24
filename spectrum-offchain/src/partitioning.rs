use std::collections::hash_map::DefaultHasher;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// Partitioned resource `R`.
/// `K` - partitioning key;
/// `N` - number of partitions.
#[derive(Debug, Clone)]
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

    pub fn new_unsafe(partitions: Vec<R>) -> Self
    where
        R: Debug,
    {
        Self {
            inner: <[R; N]>::try_from(partitions).unwrap(),
            pd: PhantomData::default(),
        }
    }
}

impl<const N: usize, R> Partitioned<N, usize, R> {
    pub fn get_by_id(&mut self, id: usize) -> &R {
        &self.inner[id % N]
    }
    pub fn get_by_id_mut(&mut self, id: usize) -> &mut R {
        &mut self.inner[id % N]
    }
}

impl<const N: usize, K, R> Partitioned<N, K, R>
where
    K: Hash,
{
    pub fn get(&self, key: K) -> &R {
        &self.inner[(hash_partitioning_key(key) % N as u64) as usize]
    }

    pub fn get_mut(&mut self, key: K) -> &mut R {
        &mut self.inner[(hash_partitioning_key(key) % N as u64) as usize]
    }
}

pub fn hash_partitioning_key<K: Hash>(key: K) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_partitioning_key_deterministic() {
        let key = "test_key";
        let hash1 = hash_partitioning_key(key);
        let hash2 = hash_partitioning_key(key);
        assert_eq!(hash1, hash2, "Hash values are not deterministic");
    }
}
