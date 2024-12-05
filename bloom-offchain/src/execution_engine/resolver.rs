use spectrum_offchain::domain::event::{Confirmed, Predicted, Unconfirmed};
use spectrum_offchain::domain::EntitySnapshot;

use crate::execution_engine::storage::StateIndex;

/// Get latest state of an on-chain entity `TEntity`.
pub fn resolve_state<T, Index>(id: T::StableId, index: &Index) -> Option<T>
where
    Index: StateIndex<T>,
    T: EntitySnapshot,
    T::StableId: Copy,
{
    index
        .get_last_predicted(id)
        .map(|Predicted(u)| u)
        .or_else(|| index.get_last_unconfirmed(id).map(|Unconfirmed(u)| u))
        .or_else(|| index.get_fallback(id))
        .or_else(|| index.get_last_confirmed(id).map(|Confirmed(u)| u))
}
