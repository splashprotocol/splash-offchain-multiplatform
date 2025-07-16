use crate::events::OnChainEvent;
use crate::queue::{QueueCmd, StrikeTime, TaskQueue};
use crate::task::{Task, TaskId};
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use futures::Stream;
use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct Executor<E, Q> {
    event_stream: E,
    queue: Q,
    current_task: Option<Pin<Box<dyn Future<Output = ControlFlow<(), ()>>>>>,
}

impl<E, Q> Executor<E, Q> {
    fn block_on(&mut self, task: impl Future<Output = ControlFlow<(), ()>> + 'static) {
        self.current_task = Some(Box::pin(task));
    }
}

impl<GaugeId, StateId, Bearer, E, Q> Future for Executor<E, Q>
where
    GaugeId: Copy + Unpin + 'static,
    StateId: Copy + Into<TaskId> + Unpin + 'static,
    Bearer: Unpin + Send + 'static,
    E: Stream<Item = (BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>, TransactionHandle)> + Unpin,
    Q: TaskQueue<TaskId, Task<GaugeId, StateId>> + Clone + Unpin + 'static,
{
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        loop {
            if let Some(mut task) = self.current_task.as_mut() {
                if let Poll::Ready(cf) = Future::poll(Pin::new(&mut task), cx) {
                    self.current_task = None;
                    if cf.is_break() {
                        break;
                    }
                } else {
                    break;
                }
            }
            let queue = self.queue.clone();
            if let Poll::Ready(Some((events, tx))) = Stream::poll_next(Pin::new(&mut self.event_stream), cx) {
                self.block_on(process_events(queue, events, tx));
                continue;
            }
            self.block_on(process_tasks(queue));
            continue;
        }
        Poll::Pending
    }
}

async fn process_events<GaugeId, StateId, Bearer, Q>(
    queue: Q,
    events: BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
    tx: TransactionHandle,
) -> ControlFlow<(), ()> where GaugeId: Copy, StateId: Copy + Into<TaskId>, Q: TaskQueue<TaskId, Task<GaugeId, StateId>> {
    let commands = match events {
        BlockEvents::RollForward {
            events,
            block_slot, ..
        } => {
            events.into_iter().filter_map(|event| match event {
                OnChainEvent::NewHarvestRequest(harvest) => {
                    Some(QueueCmd::Schedule(
                        harvest.id.into(),
                        Task::new_harvesting(harvest.id),
                        StrikeTime::Ready,
                    ))
                }
                OnChainEvent::HarvestRequestCancelled(harvest_id) => {
                    Some(QueueCmd::Cancel(harvest_id.into()))
                }
                OnChainEvent::Harvested(harvest_id) => {
                    Some(QueueCmd::Done(harvest_id.into()))
                }
                _ => None,
            }).chain(vec![QueueCmd::AdvanceClocks(block_slot)]).collect()
        }
        BlockEvents::RollBackward {
            events,
            block_slot, ..
        } => {
            events.into_iter().filter_map(|event| match event {
                OnChainEvent::NewHarvestRequest(harvest) => {
                    Some(QueueCmd::Schedule(
                        harvest.id.into(),
                        Task::new_harvesting(harvest.id),
                        StrikeTime::Ready,
                    ))
                }
                OnChainEvent::HarvestRequestCancelled(harvest_id) => {
                    Some(QueueCmd::Cancel(harvest_id.into()))
                }
                OnChainEvent::Harvested(harvest_id) => {
                    Some(QueueCmd::Done(harvest_id.into()))
                }
                _ => None,
            }).chain(vec![QueueCmd::DowngradeClocks(block_slot)]).collect()
        }
    };
    queue.batch_execute(commands).await;
    tx.commit();
    ControlFlow::Continue(())
}

async fn process_tasks<GaugeId, StateId, Q: TaskQueue<TaskId, Task<GaugeId, StateId>>>(queue: Q) -> ControlFlow<(), ()> {
    if let Some(task) = queue.first().await {
        ControlFlow::Continue(())
    } else {
        ControlFlow::Break(())   
    }
}
