use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Control<TaskId> {
    Next,
    Done(Vec<TaskId>),
}

#[async_trait]
pub trait BatchExecutor<TaskId, Task, E> {
    async fn execute(&mut self, task: Task) -> Result<Control<TaskId>, E>;
}
