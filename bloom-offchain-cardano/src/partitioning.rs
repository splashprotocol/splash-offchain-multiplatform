use std::hash::Hash;

use futures::{future, Stream, StreamExt};

use bloom_offchain::partitioning::Partitioning;

pub fn select_partition<S: Stream<Item = (Pair, T)>, Pair: Copy + Hash, T>(
    upstream: S,
    partitioning: Partitioning,
) -> impl Stream<Item = (Pair, T)> {
    upstream.filter(move |(pair, _)| future::ready(partitioning.in_my_partition(*pair)))
}
