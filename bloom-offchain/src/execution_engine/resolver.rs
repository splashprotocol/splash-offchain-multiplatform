use spectrum_offchain::data::event::{Confirmed, Predicted, Unconfirmed};
use spectrum_offchain::data::EntitySnapshot;

use crate::execution_engine::storage::StateIndex;

/// Get latest state of an on-chain entity `TEntity`.
pub fn resolve_source_state<Src, Index>(id: Src::StableId, index: &Index) -> Option<Src>
where
    Index: StateIndex<Src>,
    Src: EntitySnapshot,
    Src::StableId: Copy,
{
    index
        .get_last_predicted(id)
        .map(|Predicted(u)| u)
        .or_else(|| index.get_last_unconfirmed(id).map(|Unconfirmed(u)| u))
        .or_else(|| index.get_last_confirmed(id).map(|Confirmed(u)| u))
}
