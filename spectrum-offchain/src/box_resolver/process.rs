use std::fmt::Display;
use std::sync::Arc;

use futures::{Stream, StreamExt};
use log::trace;
use tokio::sync::Mutex;

use crate::box_resolver::persistence::EntityRepo;
use crate::data::ior::Ior;
use crate::domain::event::{Channel, Confirmed, Predicted, Transition, Unconfirmed};
use crate::domain::EntitySnapshot;
use crate::partitioning::Partitioned;

pub fn pool_tracking_stream<'a, const N: usize, S, Repo, Pool, Meta>(
    upstream: S,
    pools: Partitioned<N, Pool::StableId, Arc<Mutex<Repo>>>,
) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = Channel<Transition<Pool>, Meta>> + 'a,
    Pool: EntitySnapshot + 'a,
    Pool::StableId: Display,
    Repo: EntityRepo<Pool> + 'a,
    Meta: 'a,
{
    let pools = Arc::new(pools);
    upstream.then(move |upd_in_mode| {
        let pools = Arc::clone(&pools);
        async move {
            let is_confirmed = matches!(upd_in_mode, Channel::Ledger(_, _));
            let (Channel::Ledger(Confirmed(upd), _)
            | Channel::Mempool(Unconfirmed(upd))
            | Channel::LocalTxSubmit(Predicted(upd))) = upd_in_mode;
            match upd {
                Transition::Forward(Ior::Right(new_state))
                | Transition::Forward(Ior::Both(_, new_state))
                | Transition::Backward(Ior::Right(new_state))
                | Transition::Backward(Ior::Both(_, new_state)) => {
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
                Transition::Forward(Ior::Left(st)) => {
                    let pools_mux = pools.get(st.stable_id());
                    let mut repo = pools_mux.lock().await;
                    repo.eliminate(st).await
                }
                Transition::Backward(Ior::Left(st)) => {
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
