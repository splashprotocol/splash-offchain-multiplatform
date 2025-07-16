use crate::events::OnChainEvent;
use crate::queue::{StrikeTime, TaskQueue};
use crate::task::{Task, TaskId};
use futures::Stream;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct Executor<GaugeId, StateId, U> {
    queue: TaskQueue<TaskId, Task<GaugeId, StateId>>,
    current_task: Option<Pin<Box<dyn Future<Output = ()>>>>,
    upstream: U,
}

impl<GaugeId, StateId, U> Executor<GaugeId, StateId, U> {
    fn block_on(&mut self, task: impl Future<Output = ()> + 'static) {
        self.current_task = Some(Box::pin(task));
    }
}

impl<GaugeId, StateId, Bearer, U> Future for Executor<GaugeId, StateId, U>
where
    GaugeId: Copy + Unpin + 'static,
    StateId: Copy + Into<TaskId> + Unpin + 'static,
    Bearer: Unpin + Send + 'static,
    U: Stream<Item = OnChainEvent<GaugeId, StateId, Bearer>> + Unpin,
{
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        loop {
            if let Some(mut task) = self.current_task.as_mut() {
                if let Poll::Ready(_) = Future::poll(Pin::new(&mut task), cx) {
                    self.current_task = None;
                } else {
                    break;
                }
            }
            if let Poll::Ready(Some(event)) = Stream::poll_next(Pin::new(&mut self.upstream), cx) {
                match event {
                    OnChainEvent::NewHarvestRequest(harvest) => {
                        let task = self.queue.clone().schedule(harvest.id.into(), Task::new_harvesting(harvest.id), StrikeTime::Ready);
                        self.block_on(task);
                    }
                    OnChainEvent::HarvestRequestCancelled(harvest_id) => {}
                    OnChainEvent::Harvested(harvest_id) => {}
                    OnChainEvent::BufferWalletUpdated(update) => {}
                    OnChainEvent::GaugeUpdated(update) => {}
                    OnChainEvent::AuthManagerUpdated(update) => {}
                }
            }
            break;
        }
        Poll::Pending
    }
}
