use std::fmt::Debug;
use std::fmt::Display;
use std::marker::PhantomData;
use std::sync::{Arc, Once};
use std::time::Duration;

use async_trait::async_trait;
use futures::{stream, Stream};
use futures_timer::Delay;
use log::{info, trace, warn};
use serde::Serialize;
use tokio::sync::Mutex;
use type_equalities::{trivial_eq, IsEqual};

use crate::backlog::HotBacklog;
use crate::box_resolver::persistence::EntityRepo;
use crate::box_resolver::resolve_entity_state;
use crate::domain::event::{Predicted, Traced};
use crate::domain::order::SpecializedOrder;
use crate::domain::EntitySnapshot;
use crate::executor::RunOrderError::{Fatal, NonFatal};
use crate::executor::TxSubmissionError::{OrderUtxoIsSpent, PoolUtxoIsSpent, UnknownError};
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
    pub fn raw_builder_error(description: String, order: O) -> RunOrderError<O> {
        Fatal(description, order)
    }

    pub fn map<F, O2>(self, f: F) -> RunOrderError<O2>
    where
        F: FnOnce(O) -> O2,
    {
        match self {
            Fatal(rn, o) => Fatal(rn, f(o)),
            NonFatal(rn, o) => NonFatal(rn, f(o)),
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

#[derive(Debug, Clone, PartialEq)]
pub enum TxSubmissionError {
    PoolUtxoIsSpent,
    OrderUtxoIsSpent,
    UnknownError { info: String },
}

// Temporal solution for handling errors from cardano node socket.
// Based on attempt of extracting badInputsUtxo error from Err
// display instance. In case of submission through socket - display
// instance is just hex encoded string with error response.
// Due to impossibility of guarantee that all '25820 + 32 bytes'
// patterns represent badInputsUtxo case - we can not get error
// directly after parsing response
fn process_tx_rejected_error<'a, Err: Display, PoolVersion: Display, OrderVersion: Display>(
    tx_error: Err,
    prev_pool_version: PoolVersion,
    order_version: OrderVersion,
) -> Vec<TxSubmissionError> {
    let prefix: String = "25820".to_string(); // prefix of cbor set start?
    let parsed_error = format!("{}", tx_error);
    // string like: 25820 + tx_hash_without_index
    let pool_tx = prefix.clone()
        + (prev_pool_version.to_string().split("#").collect::<Vec<&str>>())
            .first()
            .unwrap();
    let order_tx = prefix.clone()
        + (order_version.to_string().split("#").collect::<Vec<&str>>())
            .first()
            .unwrap();

    let patterns: [(String, TxSubmissionError); 2] =
        [(pool_tx, PoolUtxoIsSpent), (order_tx, OrderUtxoIsSpent)];

    let parsed_errors = patterns
        .iter()
        .flat_map(|(tx_pattern, error)| {
            if parsed_error.contains(tx_pattern) {
                Some(error.clone())
            } else {
                None
            }
        })
        .collect();

    if parsed_error.len() > 0 {
        parsed_errors
    } else {
        // case when errors are not PoolUtxoIsSpent or OrderUtxoIsSpent
        vec![UnknownError { info: parsed_error }]
    }
}

#[async_trait(? Send)]
impl<Net, Backlog, Pools, Prover, Ctx, Ord, Pool, TxCandidate, Tx, Err> Executor
    for HotOrderExecutor<Net, Backlog, Pools, Prover, Ctx, Ord, Pool, TxCandidate, Tx, Err>
where
    Ord: SpecializedOrder + Clone + Display,
    <Ord as SpecializedOrder>::TOrderId: Clone + Display,
    Pool: EntitySnapshot + RunOrder<Ord, Ctx, TxCandidate> + Clone,
    Pool::StableId: Copy,
    Ord::TPoolId: IsEqual<Pool::StableId> + Display,
    Net: Network<Tx, Err>,
    Backlog: HotBacklog<Ord>,
    Pools: EntityRepo<Pool>,
    Prover: TxProver<TxCandidate, Tx>,
    Ctx: Clone,
    Err: Display,
    Tx: Serialize,
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
                let pool_id = entity.stable_id();
                let pool_state_id = entity.version();
                match entity.clone().try_run(ord.clone(), self.ctx.clone()) {
                    Ok((tx_candidate, next_entity_state)) => {
                        let mut entity_repo = self.pool_repo.lock().await;
                        let tx = self.prover.prove(tx_candidate);
                        if let Err(err) = self.network.submit_tx(tx).await {
                            let errors =
                                process_tx_rejected_error(err, entity.clone().version(), ord.get_self_ref());
                            warn!("Failed to submit TX. Errors {:?}", errors);
                            if !errors.is_empty() {
                                if errors.contains(&PoolUtxoIsSpent) && errors.contains(&OrderUtxoIsSpent) {
                                    entity_repo.invalidate(pool_state_id, pool_id).await;
                                } else if errors.contains(&PoolUtxoIsSpent) {
                                    entity_repo.invalidate(pool_state_id, pool_id).await;
                                    self.backlog.lock().await.put(ord);
                                }
                            }
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
            info!("Pool {} not found in storage", entity_id);
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
