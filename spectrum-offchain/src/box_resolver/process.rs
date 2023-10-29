use std::fmt::Display;
use std::sync::Arc;

use futures::{Stream, StreamExt};
use log::trace;
use tokio::sync::Mutex;

use crate::box_resolver::persistence::EntityRepo;
use crate::combinators::EitherOrBoth;
use crate::data::OnChainEntity;
use crate::data::unique_entity::{Confirmed, EitherMod, StateUpdate, Unconfirmed};
use crate::partitioning::Partitioned;

pub fn pool_tracking_stream<'a, const N: usize, S, Repo, Pool>(
    upstream: S,
    pools: Partitioned<N, Pool::Id, Arc<Mutex<Repo>>>,
) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = EitherMod<StateUpdate<Pool>>> + 'a,
    Pool: OnChainEntity + 'a,
    Pool::Id: Display,
    Repo: EntityRepo<Pool> + 'a,
{
    let pools = Arc::new(pools);
    upstream.then(move |upd_in_mode| {
        let pools = Arc::clone(&pools);
        async move {
            let is_confirmed = matches!(upd_in_mode, EitherMod::Confirmed(_));
            let (EitherMod::Confirmed(Confirmed(upd)) | EitherMod::Unconfirmed(Unconfirmed(upd))) =
                upd_in_mode;
            match upd {
                StateUpdate::Transition(EitherOrBoth::Right(new_state))
                | StateUpdate::Transition(EitherOrBoth::Both(_, new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Right(new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Both(_, new_state)) => {
                    let pool_ref = new_state.get_id();
                    let pools_mux = pools.get(pool_ref);
                    let mut repo = pools_mux.lock().await;
                    if is_confirmed {
                        trace!("Observing new confirmed state of pool {}", pool_ref);
                        repo.put_confirmed(Confirmed(new_state)).await
                    } else {
                        trace!("Observing new unconfirmed state of pool {}", pool_ref);
                        repo.put_unconfirmed(Unconfirmed(new_state)).await
                    }
                }
                StateUpdate::Transition(EitherOrBoth::Left(st)) => {
                    let pools_mux = pools.get(st.get_id());
                    let mut repo = pools_mux.lock().await;
                    repo.eliminate(st).await
                }
                StateUpdate::TransitionRollback(EitherOrBoth::Left(st)) => {
                    let pool_ref = st.get_id();
                    trace!(target: "offchain", "Rolling back state of pool {}", pool_ref);
                    let pools_mux = pools.get(pool_ref);
                    let mut repo = pools_mux.lock().await;
                    repo.invalidate(st.get_version(), st.get_id()).await
                }
            }
        }
    })
}

pub fn pool_tracking_stream_simple<'a, S, Repo, Pool>(
    upstream: S,
    pools: Arc<Mutex<Repo>>,
) -> impl Stream<Item = ()> + 'a
    where
        S: Stream<Item = EitherMod<StateUpdate<Pool>>> + 'a,
        Pool: OnChainEntity + 'a,
        Pool::Id: Display,
        Repo: EntityRepo<Pool> + 'a,
{
    upstream.then(move |upd_in_mode| {
        let pools = Arc::clone(&pools);
        async move {
            let is_confirmed = matches!(upd_in_mode, EitherMod::Confirmed(_));
            let (EitherMod::Confirmed(Confirmed(upd)) | EitherMod::Unconfirmed(Unconfirmed(upd))) =
                upd_in_mode;
            match upd {
                StateUpdate::Transition(EitherOrBoth::Right(new_state))
                | StateUpdate::Transition(EitherOrBoth::Both(_, new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Right(new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Both(_, new_state)) => {
                    let pool_ref = new_state.get_id();
                    let mut repo = pools.lock().await;
                    if is_confirmed {
                        trace!("Observing new confirmed state of pool {}", pool_ref);
                        repo.put_confirmed(Confirmed(new_state)).await
                    } else {
                        trace!("Observing new unconfirmed state of pool {}", pool_ref);
                        repo.put_unconfirmed(Unconfirmed(new_state)).await
                    }
                }
                StateUpdate::Transition(EitherOrBoth::Left(st)) => {
                    let mut repo = pools.lock().await;
                    repo.eliminate(st).await
                }
                StateUpdate::TransitionRollback(EitherOrBoth::Left(st)) => {
                    let pool_ref = st.get_id();
                    trace!(target: "offchain", "Rolling back state of pool {}", pool_ref);
                    let mut repo = pools.lock().await;
                    repo.invalidate(st.get_version(), st.get_id()).await
                }
            }
        }
    })
}
