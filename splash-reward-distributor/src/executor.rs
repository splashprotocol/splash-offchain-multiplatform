use crate::events::OnChainEvent;
use crate::queue::{StrikeTime, RocksDB, TaskQueue};
use crate::task::{Task, TaskId};
use futures::Stream;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct Executor<U, Q> {
    queue: Q,
    current_task: Option<Pin<Box<dyn Future<Output = ()>>>>,
    upstream: U,
}

impl<U, Q> Executor<U, Q> {
    fn block_on(&mut self, task: impl Future<Output = ()> + 'static) {
        self.current_task = Some(Box::pin(task));
    }
}

impl<GaugeId, StateId, Bearer, U, Q> Future for Executor<U, Q>
where
    GaugeId: Copy + Unpin + 'static,
    StateId: Copy + Into<TaskId> + Unpin + 'static,
    Bearer: Unpin + Send + 'static,
    U: Stream<Item = OnChainEvent<GaugeId, StateId, Bearer>> + Unpin,
    Q: TaskQueue<TaskId, Task<GaugeId, StateId>> + Unpin,
{
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        loop {
            // if let Some(mut task) = self.current_task.as_mut() {
            //     if let Poll::Ready(_) = Future::poll(Pin::new(&mut task), cx) {
            //         self.current_task = None;
            //     } else {
            //         break;
            //     }
            // }
            // if let Poll::Ready(Some(event)) = Stream::poll_next(Pin::new(&mut self.upstream), cx) {
            //     match event {
            //         OnChainEvent::NewHarvestRequest(harvest) => {
            //             let task = self.queue.schedule(harvest.id.into(), Task::new_harvesting(harvest.id), StrikeTime::Ready);
            //             self.block_on(task);
            //         }
            //         OnChainEvent::HarvestRequestCancelled(harvest_id) => {}
            //         OnChainEvent::Harvested(harvest_id) => {}
            //         OnChainEvent::BufferWalletUpdated(update) => {}
            //         OnChainEvent::GaugeUpdated(update) => {}
            //         OnChainEvent::AuthManagerUpdated(update) => {}
            //     }
            // }
            break;
        }
        Poll::Pending
    }
}
