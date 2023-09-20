use std::sync::Arc;

use futures::{Stream, StreamExt};
use tokio::sync::Mutex;

use crate::box_resolver::persistence::EntityRepo;
use crate::combinators::EitherOrBoth;
use crate::data::unique_entity::{Confirmed, StateUpdate};
use crate::data::OnChainEntity;
use crate::partitioning::Partitioned;

pub fn pool_tracking_stream<'a, const N: usize, S, Repo, Pool>(
    upstream: S,
    pools: Partitioned<N, Pool::TEntityId, Arc<Mutex<Repo>>>,
) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = Confirmed<StateUpdate<Pool>>> + 'a,
    Pool: OnChainEntity + 'a,
    Repo: EntityRepo<Pool> + 'a,
{
    let pools = Arc::new(pools);
    upstream.then(move |Confirmed(upd)| {
        let pools = Arc::clone(&pools);
        async move {
            match upd {
                StateUpdate::Transition(EitherOrBoth::Right(new_state))
                | StateUpdate::Transition(EitherOrBoth::Both(_, new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Right(new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Both(_, new_state)) => {
                    let pools_mux = pools.get(new_state.get_self_ref());
                    let mut repo = pools_mux.lock().await;
                    repo.put_confirmed(Confirmed(new_state)).await
                }
                StateUpdate::Transition(EitherOrBoth::Left(st)) => {
                    let pools_mux = pools.get(st.get_self_ref());
                    let mut repo = pools_mux.lock().await;
                    repo.eliminate(st).await
                }
                StateUpdate::TransitionRollback(EitherOrBoth::Left(st)) => {
                    let pools_mux = pools.get(st.get_self_ref());
                    let mut repo = pools_mux.lock().await;
                    repo.invalidate(st.get_self_state_ref(), st.get_self_ref()).await
                }
            }
        }
    })
}
