mod batch;
pub mod executor;
pub mod queue;
pub mod resolved_tx;
mod task;
pub mod verifier;
pub mod verifier_engine;

use crate::engine::executor::{BatchExecutor, Control, Error as ExecutorError};
use crate::engine::queue::{QueueCmd, StrikeTime, TaskQueue};
use crate::engine::task::{Task, TaskId};
use async_primitives::beacon::{Beacon, Once};
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use cml_crypto::TransactionHash;
use futures::channel::mpsc::Receiver;
use futures::{Stream, StreamExt};
use log::trace;
use serde::Deserialize;
use splash_dao_offchain::routines::Slot;
use splash_yf_offchain::entities::gauge::GaugeCharge;
use splash_yf_offchain::events::OnChainEvent;
use std::fmt::Debug;
use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::time::Sleep;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineConfig {
    buffering_threshold: u64,
}

pub struct Engine<U, Q, E> {
    event_stream: U,
    queue: Q,
    executor: E,
    current_task: Option<Pin<Box<dyn Future<Output = ControlFlow<(), ()>> + Send>>>,
    dropped_unconfirmed_tx_hashes_recv: Receiver<TransactionHash>,
    /// Agent is synced with the network.
    state_synced: Beacon,
    blocker: Option<Once>,
    initial_tx_ttl_delay: Option<Pin<Box<Sleep>>>,
    conf: EngineConfig,
}

impl<U, Q, E> Engine<U, Q, E> {
    pub fn new(
        event_stream: U,
        queue: Q,
        executor: E,
        conf: EngineConfig,
        dropped_unconfirmed_tx_hashes_recv: Receiver<TransactionHash>,
        state_synced: Beacon,
        initial_tx_ttl_delay: Sleep,
    ) -> Self {
        Self {
            event_stream,
            queue,
            executor,
            current_task: None,
            conf,
            dropped_unconfirmed_tx_hashes_recv,
            state_synced,
            blocker: None,
            initial_tx_ttl_delay: Some(Box::pin(initial_tx_ttl_delay)),
        }
    }

    fn block_on(&mut self, task: impl Future<Output = ControlFlow<(), ()>> + Send + 'static) {
        self.current_task = Some(Box::pin(task));
    }
}

impl<GaugeId, StateId, Bearer, U, Q, E> Stream for Engine<U, Q, E>
where
    GaugeId: Copy + Debug + Into<TaskId> + Unpin + Send + 'static,
    StateId: Copy + Into<TaskId> + Unpin + Send + 'static,
    Bearer: Unpin + Send + 'static,
    U: Stream<
            Item = (
                BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
                TransactionHandle,
            ),
        > + Unpin,
    Q: TaskQueue<TaskId, Task<GaugeId, StateId>> + Clone + Unpin + Send + 'static,
    E: BatchExecutor<TaskId, Task<GaugeId, StateId>, TransactionHash, ExecutorError>
        + Clone
        + Unpin
        + Send
        + 'static,
{
    type Item = ();
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<()>> {
        loop {
            if let Some(mut task) = self.current_task.as_mut() {
                if let Poll::Ready(cf) = Future::poll(Pin::new(&mut task), cx) {
                    self.current_task = None;
                    return Poll::Ready(Some(()));
                    //if cf.is_break() {
                    //    break;
                    //}
                }
            }

            let queue = self.queue.clone();
            if let Poll::Ready(Some((events, tx))) = Stream::poll_next(Pin::new(&mut self.event_stream), cx) {
                let conf = self.conf;
                self.block_on(process_events(queue, events, tx, conf));
                return Poll::Ready(Some(()));
            }

            if let Poll::Ready(Some(tx_hash)) =
                Stream::poll_next(Pin::new(&mut self.dropped_unconfirmed_tx_hashes_recv), cx)
            {
                self.block_on(reschedule_tasks_from_dropped_tx(tx_hash, queue));
                return Poll::Ready(Some(()));
            }

            // Wait until initial tx TTL delay is resolved (CompleteDataLoss stressor).
            if let Some(mut initial_tx_ttl_delay) = self.initial_tx_ttl_delay.take() {
                if Future::poll(Pin::new(&mut initial_tx_ttl_delay), cx).is_pending() {
                    self.initial_tx_ttl_delay = Some(initial_tx_ttl_delay);
                } else {
                    continue;
                }
            }

            // Wait until blockers are resolved.
            if let Some(mut blocker) = self.blocker.take() {
                if Future::poll(Pin::new(&mut blocker), cx).is_pending() {
                    self.blocker = Some(blocker);
                } else {
                    return Poll::Ready(Some(()));
                }
            }

            if !self.state_synced.read() && self.blocker.is_none() {
                self.blocker = Some(self.state_synced.once(true));
                continue;
            }

            if self.current_task.is_none() && self.blocker.is_none() {
                let executor = self.executor.clone();
                self.block_on(process_tasks(queue, executor));
            }
            break;
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
    GaugeId: Copy + Into<TaskId> + Debug,
    StateId: Copy + Into<TaskId>,
    Q: TaskQueue<TaskId, Task<GaugeId, StateId>> + Clone,
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
                OnChainEvent::BotHarvestingAction { payouts, tx_hash, .. } => Some(
                    payouts
                        .into_iter()
                        .map(|(harvest_order, _)| QueueCmd::Done(harvest_order.id.into(), tx_hash))
                        .chain(std::iter::once(QueueCmd::ConfirmTx(tx_hash, Slot(block_slot))))
                        .collect(),
                ),
                OnChainEvent::BotGaugeBufferingAction {
                    drained_gauges,
                    tx_hash,
                    ..
                } => Some(
                    drained_gauges
                        .0
                        .into_iter()
                        .map(|(gauge_update, _)| {
                            let task_id = gauge_update.created.0.id.into();
                            QueueCmd::Done(task_id, tx_hash)
                        })
                        .chain(std::iter::once(QueueCmd::ConfirmTx(tx_hash, Slot(block_slot))))
                        .collect(),
                ),
                OnChainEvent::ChargeGauges(GaugeCharge { gauge_update, .. }) => {
                    Some(if gauge_update.created.0.balance >= conf.buffering_threshold {
                        let gauge_id = gauge_update.created.0.id;
                        trace!("Scheduling gauge-buffering for gauge {:?}", gauge_id);
                        vec![QueueCmd::Schedule(
                            gauge_id.into(),
                            Task::new_gauge_buffering(gauge_id),
                            StrikeTime::Ready,
                        )]
                    } else {
                        vec![]
                    })
                }
                OnChainEvent::AuthManagerUpdated(_)
                | OnChainEvent::Funding { .. }
                | OnChainEvent::CreateGauge(_)
                | OnChainEvent::CreateBufferWalletAndAuthManager { .. } => None,
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
                            )
                        })
                        .collect(),
                ),

                OnChainEvent::BotGaugeBufferingAction { drained_gauges, .. } => Some(
                    drained_gauges
                        .0
                        .into_iter()
                        .map(|(gauge_update, _)| {
                            let gauge_id = gauge_update.created.0.id;
                            QueueCmd::Schedule(
                                gauge_id.into(),
                                Task::new_gauge_buffering(gauge_id),
                                StrikeTime::Ready,
                            )
                        })
                        .collect(),
                ),

                OnChainEvent::ChargeGauges(GaugeCharge { gauge_update, .. }) => {
                    Some(if gauge_update.created.0.balance >= conf.buffering_threshold {
                        let task_id = gauge_update.created.0.id.into();
                        vec![QueueCmd::Cancel(task_id)]
                    } else {
                        vec![]
                    })
                }
                OnChainEvent::AuthManagerUpdated(_)
                | OnChainEvent::Funding { .. }
                | OnChainEvent::CreateGauge(_)
                | OnChainEvent::CreateBufferWalletAndAuthManager { .. } => None,
            })
            .flatten()
            .chain(vec![QueueCmd::DowngradeClocks(block_slot)])
            .collect(),
    };
    queue.clone().batch_execute(commands).await;
    tx.commit();
    ControlFlow::Continue(())
}

async fn process_tasks<GaugeId, StateId, Q, E>(queue: Q, mut executor: E) -> ControlFlow<(), ()>
where
    Q: TaskQueue<TaskId, Task<GaugeId, StateId>> + Clone,
    E: BatchExecutor<TaskId, Task<GaugeId, StateId>, TransactionHash, ExecutorError>,
{
    let mut invalid_tasks = vec![];
    let mut rescheduled_tasks = vec![];
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
                Control::Retry => {
                    rescheduled_tasks.push(task_id);
                    continue;
                }
                Control::Stop => {}
            }
        }
        break;
    }
    match executor.execute().await {
        Ok(res) => {
            let tx_hash = res.output;

            trace!("Executed tasks: {:?}", res);
            let commands = res
                .executed_tasks
                .into_iter()
                .map(|task_id| QueueCmd::Done(task_id, tx_hash))
                .chain(
                    rescheduled_tasks
                        .into_iter()
                        .map(|task_id| QueueCmd::Reschedule(task_id, StrikeTime::In(60))),
                )
                .chain(invalid_tasks.into_iter().map(QueueCmd::Cancel));

            trace!("Commands: {:?}", commands);
            queue.batch_execute(commands.collect()).await;
            trace!("Commands executed");
        }
        Err(ExecutorError::TxInputsAlreadySpent { failed_task_ids }) => {
            let commands = failed_task_ids
                .into_iter()
                .map(|task_id| QueueCmd::Reschedule(task_id, StrikeTime::In(60)))
                .chain(
                    rescheduled_tasks
                        .into_iter()
                        .map(|task_id| QueueCmd::Reschedule(task_id, StrikeTime::In(60))),
                )
                .chain(invalid_tasks.into_iter().map(QueueCmd::Cancel))
                .collect();
            queue.batch_execute(commands).await;
        }
        Err(_) => (),
    }
    ControlFlow::Continue(())
}

async fn reschedule_tasks_from_dropped_tx<GaugeId, StateId, Q>(
    tx_hash: TransactionHash,
    queue: Q,
) -> ControlFlow<(), ()>
where
    Q: TaskQueue<TaskId, Task<GaugeId, StateId>> + Clone,
{
    if let Some(tasks) = queue.clone().read_tasks(tx_hash).await {
        let cmds = std::iter::once(QueueCmd::DropTx(tx_hash))
            .chain(tasks.into_iter().map(|(task_id, task)| {
                // Reschedule tasks and prioritise gauge-buffering TXs
                let strike_time = match &task {
                    Task::GaugeBuffering(_) => StrikeTime::Ready,
                    Task::Harvesting(_) => StrikeTime::In(60),
                };
                QueueCmd::Reschedule(task_id, strike_time)
            }))
            .collect();
        queue.batch_execute(cmds).await;
    }
    ControlFlow::Continue(())
}
