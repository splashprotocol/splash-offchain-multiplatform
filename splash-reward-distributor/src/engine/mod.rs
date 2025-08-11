mod batch;
pub mod executor;
mod prover;
mod queue;
mod resolved_tx;
mod task;
mod verifier;
mod withdrawal;

use crate::engine::executor::{BatchExecutor, Control};
use crate::engine::queue::{QueueCmd, StrikeTime, TaskQueue};
use crate::engine::task::{Task, TaskId};
use crate::events::{EntityUpdated, OnChainEvent};
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use futures::{Stream, StreamExt};
use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    buffering_threshold: u64,
}

pub struct Engine<U, Q, E> {
    event_stream: U,
    queue: Q,
    executor: E,
    current_task: Option<Pin<Box<dyn Future<Output = ControlFlow<(), ()>>>>>,
    conf: EngineConfig,
}

impl<U, Q, E> Engine<U, Q, E> {
    fn block_on(&mut self, task: impl Future<Output = ControlFlow<(), ()>> + 'static) {
        self.current_task = Some(Box::pin(task));
    }
}

impl<GaugeId, StateId, Bearer, U, Q, E> Future for Engine<U, Q, E>
where
    GaugeId: Copy + Into<TaskId> + Unpin + 'static,
    StateId: Copy + Into<TaskId> + Unpin + 'static,
    Bearer: Unpin + Send + 'static,
    U: Stream<
            Item = (
                BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
                TransactionHandle,
            ),
        > + Unpin,
    Q: TaskQueue<TaskId, Task<GaugeId, StateId>> + Clone + Unpin + 'static,
    E: Clone + BatchExecutor<TaskId, Task<GaugeId, StateId>, (), ()> + Unpin + 'static,
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
                let conf = self.conf;
                self.block_on(process_events(queue, events, tx, conf));
                continue;
            }
            let executor = self.executor.clone();
            self.block_on(process_tasks(queue, executor));
        }
        Poll::Pending
    }
}

async fn process_events<GaugeId, StateId, Bearer, Q>(
    queue: Q,
    events: BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
    tx: TransactionHandle,
    conf: EngineConfig,
) -> ControlFlow<(), ()>
where
    GaugeId: Copy + Into<TaskId>,
    StateId: Copy + Into<TaskId>,
    Q: TaskQueue<TaskId, Task<GaugeId, StateId>>,
{
    let commands = match events {
        BlockEvents::RollForward {
            events, block_slot, ..
        } => events
            .into_iter()
            .filter_map(|event| match event {
                OnChainEvent::NewHarvestRequest(harvest) => Some(QueueCmd::Schedule(
                    harvest.id.into(),
                    Task::new_harvesting(harvest.id),
                    StrikeTime::Ready,
                )),
                OnChainEvent::HarvestRequestCancelled(harvest_id) => {
                    Some(QueueCmd::Cancel(harvest_id.into()))
                }
                OnChainEvent::Harvested(harvest_order) => Some(QueueCmd::Done(harvest_order.id.into())),
                OnChainEvent::GaugeUpdated(EntityUpdated {
                    created: (gauge, _), ..
                }) if gauge.balance >= conf.buffering_threshold => Some(QueueCmd::Schedule(
                    gauge.id.into(),
                    Task::new_gauge_buffering(gauge.id),
                    StrikeTime::Ready,
                )),
                _ => None,
            })
            .chain(vec![QueueCmd::AdvanceClocks(block_slot)])
            .collect(),
        BlockEvents::RollBackward {
            events, block_slot, ..
        } => events
            .into_iter()
            .filter_map(|event| match event {
                OnChainEvent::NewHarvestRequest(harvest) => Some(QueueCmd::Cancel(harvest.id.into())),
                OnChainEvent::HarvestRequestCancelled(harvest_id) => Some(QueueCmd::Schedule(
                    harvest_id.into(),
                    Task::new_harvesting(harvest_id),
                    StrikeTime::Ready,
                )),

                OnChainEvent::Harvested(harvest_order) => Some(QueueCmd::Schedule(
                    harvest_order.id.into(),
                    Task::new_harvesting(harvest_order.id),
                    StrikeTime::Ready,
                )),
                _ => None,
            })
            .chain(vec![QueueCmd::DowngradeClocks(block_slot)])
            .collect(),
    };
    queue.batch_execute(commands).await;
    tx.commit();
    ControlFlow::Continue(())
}

async fn process_tasks<GaugeId, StateId, Q, E>(queue: Q, mut executor: E) -> ControlFlow<(), ()>
where
    Q: TaskQueue<TaskId, Task<GaugeId, StateId>> + Clone,
    E: BatchExecutor<TaskId, Task<GaugeId, StateId>, (), ()>,
{
    let mut invalid_tasks = vec![];
    let mut stream = queue.clone().pending_stream();
    loop {
        if let Some((task_id, task)) = stream.next().await {
            match executor.feed(task_id, task).await {
                Control::Drop(tid) => {
                    invalid_tasks.push(tid);
                    continue;
                }
                Control::Next => {
                    continue;
                }
                Control::Stop => {}
            }
        }
        break;
    }
    if let Ok(res) = executor.execute().await {
        let commands = res
            .executed_tasks
            .into_iter()
            .map(QueueCmd::Done)
            .chain(invalid_tasks.into_iter().map(QueueCmd::Cancel));
        queue.batch_execute(commands.collect()).await;
    }
    ControlFlow::Continue(())
}
