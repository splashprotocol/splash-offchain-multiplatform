use crate::event::LpEvent;
use async_trait::async_trait;

#[async_trait]
pub trait EventLog {
    async fn batch_append(&self, events: Vec<LpEvent>);
    async fn batch_discard(&self, events: Vec<LpEvent>);
}
