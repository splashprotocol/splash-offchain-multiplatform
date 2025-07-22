use crate::engine::task::{Harvesting, Task, TaskId};
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Control<TaskId> {
    /// Task should be dropped
    Drop(TaskId),
    /// Ready to accept at least one more task
    Next,
    ///
    Done,
}

#[derive(Debug)]
pub struct ExecutionResult<TaskId, Out> {
    pub executed_tasks: Vec<TaskId>,
    pub output: Out,
}

#[async_trait]
pub trait BatchExecutor<TaskId, Task, Out, Err> {
    async fn feed(&mut self, task: Task) -> Control<TaskId>;
    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, Out>, Err>;
}

pub struct HarvestFlow<PositionIndex, OnChainIndex> {
    position_index: PositionIndex,
    onchain_index: OnChainIndex,
}

#[async_trait]
impl<OrderId, Tx, PositionIndex, OnChainIndex> BatchExecutor<TaskId, Harvesting<OrderId>, Tx, ()>
    for HarvestFlow<PositionIndex, OnChainIndex>
where
    OrderId: Send + 'static,
    Tx: Send,
    PositionIndex: Send,
    OnChainIndex: Send,
{
    async fn feed(&mut self, task: Harvesting<OrderId>) -> Control<TaskId> {
        todo!()
    }

    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, Tx>, ()> {
        todo!()
    }
}

pub struct BufferingFlow<OnChainIndex> {
    onchain_index: OnChainIndex,
}

#[async_trait]
impl<OrderId, Tx, OnChainIndex> BatchExecutor<TaskId, Harvesting<OrderId>, Tx, ()>
    for BufferingFlow<OnChainIndex>
where
    OrderId: Send + 'static,
    Tx: Send,
    OnChainIndex: Send,
{
    async fn feed(&mut self, task: Harvesting<OrderId>) -> Control<TaskId> {
        todo!()
    }

    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, Tx>, ()> {
        todo!()
    }
}

pub struct Executor<PositionIndex, OnChainIndex, TxSubmit> {
    position_index: PositionIndex,
    onchain_index: OnChainIndex,
    tx_submit: TxSubmit,
}

#[async_trait]
impl<GaugeId, OrderId, PositionIndex, OnChainIndex, TxSubmit>
    BatchExecutor<TaskId, Task<GaugeId, OrderId>, (), ()> for Executor<PositionIndex, OnChainIndex, TxSubmit>
where
    GaugeId: Send + 'static,
    OrderId: Send + 'static,
    PositionIndex: Send,
    OnChainIndex: Send,
    TxSubmit: Send,
{
    async fn feed(&mut self, task: Task<GaugeId, OrderId>) -> Control<TaskId> {
        todo!()
    }

    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, ()>, ()> {
        todo!()
    }
}
