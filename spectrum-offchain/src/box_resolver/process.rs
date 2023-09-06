use std::sync::Arc;

use futures::{Stream, StreamExt};
use tokio::sync::Mutex;

use crate::box_resolver::persistence::EntityRepo;
use crate::combinators::EitherOrBoth;
use crate::data::unique_entity::{Confirmed, StateUpdate};
use crate::data::OnChainEntity;
use crate::partitioning::Partitioned;

pub fn entity_tracking_stream<'a, const N: usize, S, TRepo, TEntity>(
    upstream: S,
    entities: Arc<Partitioned<N, TEntity::TEntityId, Mutex<TRepo>>>,
) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = Confirmed<StateUpdate<TEntity>>> + 'a,
    TEntity: OnChainEntity + 'a,
    TRepo: EntityRepo<TEntity> + 'a,
{
    upstream.then(move |Confirmed(upd)| {
        let entities = Arc::clone(&entities);
        async move {
            match upd {
                StateUpdate::Transition(EitherOrBoth::Right(new_state))
                | StateUpdate::Transition(EitherOrBoth::Both(_, new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Right(new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Both(_, new_state)) => {
                    let entities_mux = entities.get(new_state.get_self_ref());
                    let mut repo = entities_mux.lock().await;
                    repo.put_confirmed(Confirmed(new_state)).await
                }
                StateUpdate::Transition(EitherOrBoth::Left(st)) => {
                    let entities_mux = entities.get(st.get_self_ref());
                    let mut repo = entities_mux.lock().await;
                    repo.eliminate(st).await
                }
                StateUpdate::TransitionRollback(EitherOrBoth::Left(st)) => {
                    let entities_mux = entities.get(st.get_self_ref());
                    let mut repo = entities_mux.lock().await;
                    repo.invalidate(st.get_self_state_ref(), st.get_self_ref()).await
                }
            }
        }
    })
}
