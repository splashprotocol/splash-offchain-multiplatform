mod cooperative;
mod flow;

use async_trait::async_trait;

#[async_trait]
pub trait TaskManagement<PartId, TaskId, Task> {
    async fn enqueue(&self, partition: PartId, task: Task);
}
