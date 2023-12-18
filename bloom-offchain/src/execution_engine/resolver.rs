use spectrum_offchain::data::LiquiditySource;
use spectrum_offchain::data::unique_entity::{Confirmed, Predicted, Unconfirmed};

use crate::execution_engine::storage::StateIndex;

/// Get latest state of an on-chain entity `TEntity`.
pub fn resolve_source_state<Src, Index>(
    id: Src::StableId,
    index: &Index,
) -> Option<Src>
    where
        Index: StateIndex<Src>,
        Src: LiquiditySource,
        Src::StableId: Copy,
{
    let states = {
        let confirmed = index.get_last_confirmed(id);
        let unconfirmed = index.get_last_unconfirmed(id);
        let predicted = index.get_last_predicted(id);
        (confirmed, unconfirmed, predicted)
    };
    match states {
        (Some(Confirmed(conf)), unconf, Some(Predicted(pred))) => {
            let anchoring_point = unconf.map(|Unconfirmed(e)| e).unwrap_or(conf);
            let anchoring_sid = anchoring_point.version();
            let predicted_sid = pred.version();
            let prediction_is_anchoring_point = predicted_sid == anchoring_sid;
            let prediction_is_valid = prediction_is_anchoring_point
                || is_linking(predicted_sid, anchoring_sid, index);
            let safe_point = if prediction_is_valid {
                pred
            } else {
                anchoring_point
            };
            Some(safe_point)
        }
        (_, Some(Unconfirmed(unconf)), None) => Some(unconf),
        (Some(Confirmed(conf)), _, _) => Some(conf),
        _ => None,
    }
}

fn is_linking<Src, Index>(
    ver: Src::Version,
    anchoring_ver: Src::Version,
    index: &Index,
) -> bool
    where
        Src: LiquiditySource,
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
