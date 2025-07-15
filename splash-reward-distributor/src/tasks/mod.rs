mod cooperative;
mod flow;
mod store;

use async_trait::async_trait;
use crate::tasks::store::TaskStore;

#[async_trait]
pub trait TaskManagement<PartId, TaskId, Task> {
    async fn enqueue(&self, partition: PartId, task: Task);
}

pub struct TaskQueue<PartId, TaskId, Task> {
    store: TaskStore<PartId, TaskId, Task>
}
