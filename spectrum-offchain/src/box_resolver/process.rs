use std::sync::Arc;

use futures::{Stream, StreamExt};
use tokio::sync::Mutex;

use crate::box_resolver::persistence::EntityRepo;
use crate::combinators::EitherOrBoth;
use crate::data::unique_entity::{Confirmed, StateUpdate};
use crate::data::OnChainEntity;

pub fn entity_tracking_stream<'a, S, TRepo, TEntity>(
    upstream: S,
    entities: Arc<Mutex<TRepo>>,
) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = Confirmed<StateUpdate<TEntity>>> + 'a,
    TEntity: OnChainEntity + 'a,
    TRepo: EntityRepo<TEntity> + 'a,
{
    upstream.then(move |Confirmed(upd)| {
        let entities = Arc::clone(&entities);
        async move {
            let mut repo = entities.lock().await;
            match upd {
                StateUpdate::Transition(EitherOrBoth::Right(new_state))
                | StateUpdate::Transition(EitherOrBoth::Both(_, new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Right(new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Both(_, new_state)) => {
                    repo.put_confirmed(Confirmed(new_state)).await
                }
                StateUpdate::Transition(EitherOrBoth::Left(st)) => repo.eliminate(st).await,
                StateUpdate::TransitionRollback(EitherOrBoth::Left(st)) => {
                    repo.invalidate(st.get_self_state_ref(), st.get_self_ref()).await
                }
            }
        }
    })
}
