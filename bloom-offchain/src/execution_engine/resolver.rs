use spectrum_offchain::data::unique_entity::{Confirmed, Predicted, Unconfirmed};
use spectrum_offchain::data::EntitySnapshot;

use crate::execution_engine::storage::StateIndex;

/// Get latest state of an on-chain entity `TEntity`.
pub fn resolve_source_state<Src, Index>(id: Src::StableId, index: &Index) -> Option<Src>
where
    Index: StateIndex<Src>,
    Src: EntitySnapshot,
    Src::StableId: Copy,
{
    let states = {
        let confirmed = index.get_last_confirmed(id);
        let unconfirmed = index.get_last_unconfirmed(id);
        (confirmed, unconfirmed)
    };
    match states {
        (_, Some(Unconfirmed(unconfirmed))) => Some(unconfirmed),
        (Some(Confirmed(confirmed)), _) => Some(confirmed),
        _ => None,
    }
}

fn is_linking<Src, Index>(ver: Src::Version, anchoring_ver: Src::Version, index: &Index) -> bool
where
    Src: EntitySnapshot,
    Index: StateIndex<Src>,
{
    let mut head_sid = ver;
    loop {
        match index.get_prediction_predecessor(head_sid) {
            None => return false,
            Some(prev_state_id) if prev_state_id == anchoring_ver => return true,
            Some(prev_state_id) => head_sid = prev_state_id,
        }
    }
}
