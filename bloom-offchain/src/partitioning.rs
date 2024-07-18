use std::hash::Hash;

use spectrum_offchain::partitioning::hash_partitioning_key;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Partitioning {
    pub num_partitions_total: u64,
    pub assigned_partitions: Vec<u64>,
}

impl Partitioning {
    pub fn in_my_partition<K: Hash>(&self, k: K) -> bool {
        let part = hash_partitioning_key(k) % self.num_partitions_total;
        self.assigned_partitions.contains(&part)
    }
}
