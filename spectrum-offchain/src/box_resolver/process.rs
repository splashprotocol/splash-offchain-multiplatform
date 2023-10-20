use std::fmt::Display;
use std::sync::Arc;

use futures::{Stream, StreamExt};
use log::trace;
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
    Pool::TEntityId: Display,
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
                    let pool_ref = new_state.get_self_ref();
                    trace!(target: "offchain", "Observing new state of pool {}", pool_ref);
                    let pools_mux = pools.get(pool_ref);
                    let mut repo = pools_mux.lock().await;
                    repo.put_confirmed(Confirmed(new_state)).await
                }
                StateUpdate::Transition(EitherOrBoth::Left(st)) => {
                    let pools_mux = pools.get(st.get_self_ref());
                    let mut repo = pools_mux.lock().await;
                    repo.eliminate(st).await
                }
                StateUpdate::TransitionRollback(EitherOrBoth::Left(st)) => {
                    let pool_ref = st.get_self_ref();
                    trace!(target: "offchain", "Rolling back state of pool {}", pool_ref);
                    let pools_mux = pools.get(pool_ref);
                    let mut repo = pools_mux.lock().await;
                    repo.invalidate(st.get_self_state_ref(), st.get_self_ref()).await
                }
            }
        }
    })
}
