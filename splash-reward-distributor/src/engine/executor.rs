use std::fmt::Display;
use crate::engine::batch::{BufferingBatch, HarvestBatch};
use crate::engine::task::{Harvesting, Task, TaskId};
use crate::index::HarvestingIndex;
use async_trait::async_trait;
use log::{error, warn};
use spectrum_offchain::network::Network;
use std::marker::PhantomData;
use crate::positions::{AccountLocked, LockAccountRejection, Positions};

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

pub struct HarvestFlow<StateId, Bearer, PositionIndex, OnChainIndex> {
    position_index: PositionIndex,
    onchain_index: OnChainIndex,
    batch: Option<HarvestBatch<StateId, Bearer>>,
}

#[async_trait]
impl<StateId, Bearer, Tx, PositionIndex, OnChainIndex> BatchExecutor<TaskId, Harvesting<StateId>, Tx, ()>
    for HarvestFlow<StateId, Bearer, PositionIndex, OnChainIndex>
where
    StateId: Send + Sync + Display + 'static,
    Bearer: Send,
    Tx: Send,
    PositionIndex: Positions<StateId> + Send,
    OnChainIndex: HarvestingIndex<StateId, Bearer> + Send,
{
    async fn feed(&mut self, task_id: TaskId, task: Harvesting<StateId>) -> Control<TaskId> {
        let order = if let Some(order) = self.onchain_index.get_order(task.order_id).await {
            order
        } else {
            return Control::Drop(task_id);
        };
        if let Some(ref mut batch) = self.batch {
            if let Err(_) = batch.try_add_order(order) {
                return Control::Stop;
            }
        } else {
            if let Some(bw) = self.onchain_index.get_buffer_wallet().await {
                let _ = self.batch.insert(HarvestBatch::new(bw, order));
            } else {
                error!("No buffer wallet found");
                return Control::Stop;
            }
        }
        Control::Next
    }

    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, Tx>, ()> {
        if let Some(batch) = self.batch.take() {
            for req in batch.orders() {
                match self.position_index.lock_account(&req.id, &req.account).await {
                    Ok(locked) => {
                        // compute abs SPLASH share
                    }
                    Err(err) => {
                        warn!("Account {} already locked by another request {}", hex::encode(req.account.to_raw_bytes()), req.id);
                    }
                }
            }
            todo!()
        } else {
            Err(())
        }
    }
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
