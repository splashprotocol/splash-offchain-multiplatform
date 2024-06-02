use std::hash::Hash;

use spectrum_offchain::partitioning::hash_partitioning_key;

#[derive(Debug, Copy, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Partitioning {
    pub num_partitions_total: u64,
    pub own_partition_index: u64,
}

impl Partitioning {
    pub fn in_my_partition<K: Hash>(&self, k: K) -> bool {
        hash_partitioning_key(k) % self.num_partitions_total == self.own_partition_index
    }
}
