use std::fmt::Debug;
use std::fmt::Display;
use std::marker::PhantomData;
use std::sync::{Arc, Once};
use std::time::Duration;

use async_trait::async_trait;
use futures::{stream, Stream};
use futures_timer::Delay;
use log::trace;
use log::{info, warn};
use tokio::sync::Mutex;
use type_equalities::{trivial_eq, IsEqual};

use crate::backlog::HotBacklog;
use crate::box_resolver::persistence::EntityRepo;
use crate::box_resolver::resolve_entity_state;
use crate::data::unique_entity::{Predicted, Traced};
use crate::data::{OnChainEntity, SpecializedOrder};
use crate::network::Network;
use crate::tx_prover::TxProver;

/// Indicated the kind of failure on at attempt to execute an order offline.
#[derive(Debug, PartialEq, Eq)]
pub enum RunOrderError<TOrd> {
    /// Discard order in the case of fatal failure.
    Fatal(String, TOrd),
    /// Return order in the case of non-fatal failure.
    NonFatal(String, TOrd),
}

impl<O> RunOrderError<O> {
    pub fn map<F, O2>(self, f: F) -> RunOrderError<O2>
    where
        F: FnOnce(O) -> O2,
    {
        match self {
            RunOrderError::Fatal(rn, o) => RunOrderError::Fatal(rn, f(o)),
            RunOrderError::NonFatal(rn, o) => RunOrderError::NonFatal(rn, f(o)),
        }
    }
}

pub trait RunOrder<Order, Ctx, Tx>: Sized {
    /// Try to run `Self` against the given `TEntity`.
    /// Returns transaction and the next state of the persistent entity in the case of success.
    /// Returns `RunOrderError<TOrd>` otherwise.
    fn try_run(
        self,
        order: Order,
        ctx: Ctx, // can be used to pass extra deps
    ) -> Result<(Tx, Predicted<Self>), RunOrderError<Order>>;
}

#[async_trait(? Send)]
pub trait Executor {
    /// Execute next available order.
    /// Drives execution to completion (submit tx or handle error).
    async fn try_execute_next(&mut self) -> bool;
}

/// A generic executor suitable for cases when single order is applied to a single entity (pool).
pub struct HotOrderExecutor<Net, Backlog, Pools, Prover, Ctx, Ord, Pool, TxCandidate, Tx, Err> {
    network: Net,
    backlog: Arc<Mutex<Backlog>>,
    pool_repo: Arc<Mutex<Pools>>,
    prover: Prover,
    ctx: Ctx,
    pd1: PhantomData<Ord>,
    pd2: PhantomData<Pool>,
    pd3: PhantomData<TxCandidate>,
    pd4: PhantomData<Tx>,
    pd5: PhantomData<Err>,
}

impl<Net, Backlog, Pools, Prover, Ctx, Ord, Pool, TxCandidate, Tx, Err>
    HotOrderExecutor<Net, Backlog, Pools, Prover, Ctx, Ord, Pool, TxCandidate, Tx, Err>
{
    pub fn new(
        network: Net,
        backlog: Arc<Mutex<Backlog>>,
        pool_repo: Arc<Mutex<Pools>>,
        prover: Prover,
        ctx: Ctx,
    ) -> Self {
        Self {
            network,
            backlog,
            pool_repo,
            prover,
            ctx,
            pd1: Default::default(),
            pd2: Default::default(),
            pd3: Default::default(),
            pd4: Default::default(),
            pd5: Default::default(),
        }
    }
}

#[async_trait(? Send)]
impl<Net, Backlog, Pools, Prover, Ctx, Ord, Pool, TxCandidate, Tx, Err> Executor
    for HotOrderExecutor<Net, Backlog, Pools, Prover, Ctx, Ord, Pool, TxCandidate, Tx, Err>
where
    Ord: SpecializedOrder + Clone + Display,
    <Ord as SpecializedOrder>::TOrderId: Clone + Display,
    Pool: OnChainEntity + RunOrder<Ord, Ctx, TxCandidate> + Clone,
    Pool::TEntityId: Copy,
    Ord::TPoolId: IsEqual<Pool::TEntityId> + Display,
    Net: Network<Tx, Err>,
    Backlog: HotBacklog<Ord>,
    Pools: EntityRepo<Pool>,
    Prover: TxProver<TxCandidate, Tx>,
    Ctx: Clone,
    Err: Display,
{
    async fn try_execute_next(&mut self) -> bool {
        let next_ord = {
            let mut backlog = self.backlog.lock().await;
            backlog.try_pop()
        };
        if let Some(ord) = next_ord {
            let entity_id = ord.get_pool_ref();
            info!("Running order {} against pool {}", ord.get_self_ref(), entity_id);
            if let Some(entity) =
                resolve_entity_state(trivial_eq().coerce(entity_id), Arc::clone(&self.pool_repo)).await
            {
                let pool_id = entity.get_self_ref();
                let pool_state_id = entity.get_self_state_ref();
                match entity.try_run(ord.clone(), self.ctx.clone()) {
                    Ok((tx_candidate, next_entity_state)) => {
                        let mut entity_repo = self.pool_repo.lock().await;
                        let tx = self.prover.prove(tx_candidate);
                        if let Err(err) = self.network.submit_tx(tx).await {
                            warn!("Failed to submit TX: {}", err);
                            entity_repo.invalidate(pool_state_id, pool_id).await;
                            self.backlog.lock().await.recharge(ord); // Return order to backlog
                        } else {
                            entity_repo
                                .put_predicted(Traced {
                                    state: next_entity_state,
                                    prev_state_id: Some(pool_state_id),
                                })
                                .await;
                        }
                    }
                    Err(RunOrderError::NonFatal(err, _) | RunOrderError::Fatal(err, _)) => {
                        info!("Order dropped due to fatal error: {}", err);
                    }
                }
                return true;
            }
        }
        false
    }
}

const THROTTLE_IDLE_MILLIS: u64 = 100;
const THROTTLE_PREM_MILLIS: u64 = 1000;

/// Construct Executor stream that drives sequential order execution.
pub fn executor_stream<'a, TExecutor: Executor + 'a>(
    executor: TExecutor,
    tip_reached_signal: Option<&'a Once>,
) -> impl Stream<Item = ()> + 'a {
    let executor = Arc::new(Mutex::new(executor));
    stream::unfold((), move |_| {
        let executor = executor.clone();
        async move {
            if tip_reached_signal.map(|sig| sig.is_completed()).unwrap_or(true) {
                trace!("Trying to execute next order ..");
                let mut executor_guard = executor.lock().await;
                if !executor_guard.try_execute_next().await {
                    trace!("No orders available ..");
                    Delay::new(Duration::from_millis(THROTTLE_IDLE_MILLIS)).await;
                }
            } else {
                Delay::new(Duration::from_millis(THROTTLE_PREM_MILLIS)).await;
            }
            Some(((), ()))
        }
    })
}
