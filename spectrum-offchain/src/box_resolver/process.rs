use std::fmt::Display;
use std::sync::Arc;

use futures::{Stream, StreamExt};
use log::trace;
use tokio::sync::Mutex;

use crate::box_resolver::persistence::EntityRepo;
use crate::combinators::Ior;
use crate::data::unique_entity::{Confirmed, EitherMod, StateUpdate, Unconfirmed};
use crate::data::LiquiditySource;
use crate::partitioning::Partitioned;

pub fn pool_tracking_stream<'a, const N: usize, S, Repo, Pool>(
    upstream: S,
    pools: Partitioned<N, Pool::StableId, Arc<Mutex<Repo>>>,
) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = EitherMod<StateUpdate<Pool>>> + 'a,
    Pool: LiquiditySource + 'a,
    Pool::StableId: Display,
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
                StateUpdate::Transition(Ior::Right(new_state))
                | StateUpdate::Transition(Ior::Both(_, new_state))
                | StateUpdate::TransitionRollback(Ior::Right(new_state))
                | StateUpdate::TransitionRollback(Ior::Both(_, new_state)) => {
                    let pool_ref = new_state.stable_id();
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
                StateUpdate::Transition(Ior::Left(st)) => {
                    let pools_mux = pools.get(st.stable_id());
                    let mut repo = pools_mux.lock().await;
                    repo.eliminate(st).await
                }
                StateUpdate::TransitionRollback(Ior::Left(st)) => {
                    let pool_ref = st.stable_id();
                    trace!(target: "offchain", "Rolling back state of pool {}", pool_ref);
                    let pools_mux = pools.get(pool_ref);
                    let mut repo = pools_mux.lock().await;
                    repo.invalidate(st.version(), st.stable_id()).await
                }
            }
        }
    })
}
