use crate::emission::Emission;
use crate::engine::batch::{BufferingBatch, HarvestBatch};
use crate::engine::task::{Harvesting, Task, TaskId};
use crate::index::HarvestingIndex;
use crate::positions::{AccountState, LockAccountRejection, LockedByAnotherReq, Positions};
use async_trait::async_trait;
use bloom_offchain::execution_engine::bundled::Bundled;
use log::{error, warn};
use spectrum_offchain::network::Network;
use splash_dao_offchain::entities::onchain::inflation_box::emission_rate;
use std::fmt::Display;
use std::marker::PhantomData;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Control<TaskId> {
    /// Task should be dropped
    Drop(TaskId),
    /// Ready to accept at least one more task
    Next,
    /// Batch is full
    Stop,
}

#[derive(Debug)]
pub struct ExecutionResult<TaskId, Out> {
    pub executed_tasks: Vec<TaskId>,
    pub output: Out,
}

#[async_trait]
pub trait BatchExecutor<TaskId, Task, Out, Err> {
    async fn feed(&mut self, task_id: TaskId, task: Task) -> Control<TaskId>;
    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, Out>, Err>;
}

pub struct HarvestFlow<StateId, Bearer, PositionIndex, OnChainIndex, Emission> {
    position_index: PositionIndex,
    onchain_index: OnChainIndex,
    emission: Emission,
    batch: Option<HarvestBatch<StateId, Bearer>>,
}

#[async_trait]
impl<StateId, Bearer, Tx, PositionIndex, OnChainIndex, Emiss>
    BatchExecutor<TaskId, Harvesting<StateId>, Tx, ()>
    for HarvestFlow<StateId, Bearer, PositionIndex, OnChainIndex, Emiss>
where
    StateId: Send + Sync + Display + 'static,
    Bearer: Send,
    Tx: Send,
    PositionIndex: Positions<StateId> + Send,
    OnChainIndex: HarvestingIndex<StateId, Bearer> + Send,
    Emiss: Emission + Send,
{
    async fn feed(&mut self, task_id: TaskId, task: Harvesting<StateId>) -> Control<TaskId> {
        let order = if let Some(order) = self.onchain_index.get_order(task.order_id).await {
            order
        } else {
            return Control::Drop(task_id);
        };
        let batch = if let Some(ref mut batch) = self.batch {
            batch
        } else {
            if let Some(bw) = self.onchain_index.get_buffer_wallet().await {
                self.batch.insert(HarvestBatch::new(bw))
            } else {
                error!("No buffer wallet found");
                return Control::Stop;
            }
        };
        let Bundled(req, _) = &order;
        match self.position_index.query_account(&req.account).await {
            Ok(AccountState {
                activated_at,
                queried_at,
                total_share_bps,
            }) => {
                let emission = self.emission.total_emission_between(activated_at, queried_at);
                let payout = bps_to_abs(total_share_bps, emission);
                if batch.can_accept(payout) {
                    if let Err(LockedByAnotherReq(concurrent_req)) =
                        self.position_index.lock_account(&req.id, &req.account).await
                    {
                        warn!(
                            "Account {} is already locked by another request {}, dropping request {}",
                            hex::encode(req.account.to_raw_bytes()),
                            concurrent_req,
                            req.id
                        );
                        return Control::Drop(task_id);
                    }
                    batch.add_order(order, payout);
                } else {
                    warn!(
                        "Buffer wallet is running out of funds, cannot process request {}",
                        req.id
                    );
                    return Control::Stop;
                }
            }
            Err(_not_found) => {
                warn!(
                    "Account {} not found, dropping request {}",
                    hex::encode(req.account.to_raw_bytes()),
                    req.id
                );
                return Control::Drop(task_id);
            }
        }
        Control::Next
    }

    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, Tx>, ()> {
        if let Some(batch) = self.batch.take() {
            todo!()
        } else {
            Err(())
        }
    }
}

fn bps_to_abs(bps: u64, x: u64) -> u64 {
    (bps * x) / 10_000
}

pub struct BufferingFlow<StateId, GaugeId, Bearer, OnChainIndex> {
    onchain_index: OnChainIndex,
    batch: BufferingBatch<StateId, GaugeId, Bearer>,
}

#[async_trait]
impl<StateId, GaugeId, Bearer, Tx, OnChainIndex> BatchExecutor<TaskId, Harvesting<StateId>, Tx, ()>
    for BufferingFlow<StateId, GaugeId, Bearer, OnChainIndex>
where
    StateId: Send + 'static,
    GaugeId: Send + 'static,
    Bearer: Send,
    Tx: Send,
    OnChainIndex: Send,
{
    async fn feed(&mut self, task_id: TaskId, task: Harvesting<StateId>) -> Control<TaskId> {
        todo!()
    }

    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, Tx>, ()> {
        todo!()
    }
}

pub struct Executor<Tx, TxErr, PositionIndex, OnChainIndex, TxSubmit> {
    position_index: PositionIndex,
    onchain_index: OnChainIndex,
    tx_submit: TxSubmit,
    pd: PhantomData<(Tx, TxErr)>,
}

#[async_trait]
impl<GaugeId, OrderId, Tx, TxErr, PositionIndex, OnChainIndex, TxSubmit>
    BatchExecutor<TaskId, Task<GaugeId, OrderId>, (), ()>
    for Executor<Tx, TxErr, PositionIndex, OnChainIndex, TxSubmit>
where
    GaugeId: Send + 'static,
    OrderId: Send + 'static,
    Tx: Send,
    TxErr: Send,
    PositionIndex: Send,
    OnChainIndex: Send,
    TxSubmit: Network<Tx, TxErr> + Send,
{
    async fn feed(&mut self, task_id: TaskId, task: Task<GaugeId, OrderId>) -> Control<TaskId> {
        todo!()
    }

    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, ()>, ()> {
        todo!()
    }
}
