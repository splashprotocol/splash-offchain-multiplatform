use std::sync::Arc;

use tokio::sync::Mutex;

use crate::box_resolver::persistence::EntityRepo;
use crate::domain::event::{Confirmed, Predicted, Traced, Unconfirmed};
use crate::domain::EntitySnapshot;

pub mod blacklist;
pub mod persistence;
pub mod process;

/// Get latest state of an on-chain entity `TEntity`.
pub async fn resolve_entity_state<TEntity, TRepo>(
    id: TEntity::StableId,
    repo: Arc<Mutex<TRepo>>,
) -> Option<TEntity>
where
    TRepo: EntityRepo<TEntity>,
    TEntity: EntitySnapshot,
    TEntity::StableId: Copy,
{
    let states = {
        let repo_guard = repo.lock().await;
        let confirmed = repo_guard.get_last_confirmed(id).await;
        let unconfirmed = repo_guard.get_last_unconfirmed(id).await;
        let predicted = repo_guard.get_last_predicted(id).await;
        (confirmed, unconfirmed, predicted)
    };
    match states {
        (Some(Confirmed(conf)), unconf, Some(Predicted(pred))) => {
            let anchoring_point = unconf.map(|Unconfirmed(e)| e).unwrap_or(conf);
            let anchoring_sid = anchoring_point.version();
            let predicted_sid = pred.version();
            let prediction_is_anchoring_point = predicted_sid == anchoring_sid;
            let prediction_is_valid = prediction_is_anchoring_point
                || is_linking(predicted_sid, anchoring_sid, Arc::clone(&repo)).await;
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

async fn is_linking<TEntity, TRepo>(
    sid: TEntity::Version,
    anchoring_sid: TEntity::Version,
    repo: Arc<Mutex<TRepo>>,
) -> bool
where
    TEntity: EntitySnapshot,
    TRepo: EntityRepo<TEntity>,
{
    let mut head_sid = sid;
    let repo = repo.lock().await;
    loop {
        match repo.get_prediction_predecessor(head_sid).await {
            None => return false,
            Some(prev_state_id) if prev_state_id == anchoring_sid => return true,
            Some(prev_state_id) => head_sid = prev_state_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use crate::box_resolver::persistence::tests::*;
    use crate::box_resolver::persistence::EntityRepo;
    use crate::box_resolver::resolve_entity_state;
    use crate::domain::event::Confirmed;
    use crate::domain::Stable;

    #[tokio::test]
    async fn test_resolve_state_trivial() {
        let mut client = rocks_db_client();
        let entity = Confirmed(TestEntity {
            token_id: TokenId::random(),
            box_id: BoxId::random(),
        });
        client.put_confirmed(entity.clone()).await;

        let client = Arc::new(Mutex::new(client));
        let resolved = resolve_entity_state::<TestEntity, _>(entity.0.stable_id(), client).await;
        assert_eq!(resolved, Some(entity.0));
    }
}
