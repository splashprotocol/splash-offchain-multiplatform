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
use crate::events::OnChainEvent;
use crate::onchain::smart_farm::UpdatedGauges;
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use futures::{Stream, StreamExt};
use std::fmt::Debug;
use std::future::Future;
use std::hash::Hash;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    buffering_threshold: u64,
}

pub struct Engine<U, Q, E, I> {
    event_stream: U,
    queue: Q,
    indexer: I,
    executor: E,
    current_task: Option<Pin<Box<dyn Future<Output = ControlFlow<(), ()>>>>>,
    conf: EngineConfig,
}

impl<U, Q, E, I> Engine<U, Q, E, I> {
    fn block_on(&mut self, task: impl Future<Output = ControlFlow<(), ()>> + 'static) {
        self.current_task = Some(Box::pin(task));
    }
}

impl<GaugeId, StateId, Bearer, U, Q, E, I> Future for Engine<U, Q, E, I>
where
    GaugeId: Copy + Unpin + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
    StateId: Copy
        + Into<TaskId>
        + Unpin
        + Eq
        + Hash
        + Send
        + Sync
        + Display
        + Debug
        + Serialize
        + DeserializeOwned
        + 'static,
    Bearer: Serialize + DeserializeOwned + Unpin + Send + 'static,
    I: Unpin + Send + Sync + Clone + HarvestOrderIndex<StateId, Bearer> + OnChainIndex<Bearer> + 'static,
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
            let indexer = self.indexer.clone();
            if let Poll::Ready(Some((events, tx))) = Stream::poll_next(Pin::new(&mut self.event_stream), cx) {
                self.block_on(process_events(queue, events, tx));
                continue;
            }
            let executor = self.executor.clone();
            self.block_on(process_tasks(queue, executor));
        }
        Poll::Pending
    }
}

async fn process_events<GaugeId, StateId, Bearer, Q, I>(
    queue: Q,
    events: BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
    indexer: I,
    tx: TransactionHandle,
    conf: EngineConfig,
) -> ControlFlow<(), ()>
where
    GaugeId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
    StateId: Copy
        + Into<TaskId>
        + Eq
        + Hash
        + Send
        + Sync
        + Display
        + Debug
        + Serialize
        + DeserializeOwned
        + 'static,
    I: HarvestOrderIndex<StateId, Bearer> + OnChainIndex<Bearer>,
    Bearer: Serialize + DeserializeOwned + 'static,
    Q: TaskQueue<TaskId, Task<GaugeId, StateId>>,
{
    let commands = match events {
        BlockEvents::RollForward {
            events, block_slot, ..
        } => events
            .into_iter()
            .filter_map(|event| match event {
                OnChainEvent::NewHarvestRequest(harvest, _) => {
                    let harvest_id = harvest.id;
                    let task_id = harvest_id.into();
                    Some(vec![QueueCmd::Schedule(
                        task_id,
                        Task::new_harvesting(harvest_id),
                        StrikeTime::Ready,
                    )])
                }
                OnChainEvent::HarvestRequestCancelled(harvest_ids) => Some(
                    harvest_ids
                        .into_iter()
                        .map(|id| {
                            let task_id: TaskId = id.into();
                            QueueCmd::Cancel(task_id)
                        })
                        .collect(),
                ),
                OnChainEvent::BotHarvestingAction { payouts, .. } => Some(
                    payouts
                        .into_iter()
                        .map(|(harvest_order, _)| QueueCmd::Done(harvest_order.id.into()))
                        .collect(),
                ),
                OnChainEvent::BotGaugeBufferingAction { drained_gauges, .. } => Some(
                    drained_gauges
                        .into_iter()
                        .map(|gauge_update| {
                            let task_id = gauge_update.created.0.id.into();
                            QueueCmd::Done(task_id)
                        })
                        .collect(),
                ),
                OnChainEvent::UpdatedGauges(UpdatedGauges(updated_gauges)) => Some(
                    updated_gauges
                        .into_iter()
                        .filter_map(|gauge_update| {
                            if gauge_update.created.0.balance >= conf.buffering_threshold {
                                let gauge_id = gauge_update.created.0.id;
                                return Some(QueueCmd::Schedule(
                                    gauge_id.into(),
                                    Task::new_gauge_buffering(gauge_id),
                                    StrikeTime::Ready,
                                ));
                            }
                            None
                        })
                        .collect(),
                ),
                OnChainEvent::AuthManagerUpdated(_) => None,
            })
            .flatten()
            .chain(vec![QueueCmd::AdvanceClocks(block_slot)])
            .collect(),
        BlockEvents::RollBackward {
            events, block_slot, ..
        } => events
            .into_iter()
            .filter_map(|event| match event {
                OnChainEvent::NewHarvestRequest(harvest, _) => {
                    Some(vec![QueueCmd::Cancel(harvest.id.into())])
                }
                OnChainEvent::HarvestRequestCancelled(harvest_ids) => Some(
                    harvest_ids
                        .into_iter()
                        .map(|harvest_id| {
                            QueueCmd::Schedule(
                                harvest_id.into(),
                                Task::new_harvesting(harvest_id),
                                StrikeTime::Ready,
                            )
                        })
                        .collect(),
                ),

                OnChainEvent::BotHarvestingAction { payouts, .. } => Some(
                    payouts
                        .into_iter()
                        .map(|(harvest_order, _)| {
                            QueueCmd::Schedule(
                                harvest_order.id.into(),
                                Task::new_harvesting(harvest_order.id),
                                StrikeTime::Ready,
                            ));
                        }
                    }

                    OnChainEvent::BotGaugeBufferingAction {
                        drained_gauges,
                        buffer_wallet_update,
                    } => {
                        let prev_state_id = indexer
                            .remove::<BufferWallet<_>>(
                                BufferWalletId,
                                buffer_wallet_update.created.0.state_id,
                            )
                        })
                        .collect(),
                ),

                OnChainEvent::BotGaugeBufferingAction { drained_gauges, .. } => Some(
                    drained_gauges
                        .into_iter()
                        .map(|gauge_update| {
                            let gauge_id = gauge_update.created.0.id;
                            QueueCmd::Schedule(
                                gauge_id.into(),
                                Task::new_gauge_buffering(gauge_id),
                                StrikeTime::Ready,
                            )
                        })
                        .collect(),
                ),

                OnChainEvent::UpdatedGauges(UpdatedGauges(updated_gauges)) => Some(
                    updated_gauges
                        .into_iter()
                        .filter_map(|gauge_update| {
                            if gauge_update.created.0.balance >= conf.buffering_threshold {
                                let task_id = gauge_update.created.0.id.into();
                                return Some(QueueCmd::Cancel(task_id));
                            }
                            None
                        })
                        .collect(),
                ),
                OnChainEvent::AuthManagerUpdated(_) => None,
            })
            .flatten()
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
