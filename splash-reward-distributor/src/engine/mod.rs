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
use crate::indexer::{HarvestOrderIndex, OnChainIndex};
use crate::onchain::auth_manager::{AuthManager, AuthManagerId};
use crate::onchain::buffer_wallet::{BufferWallet, BufferWalletId};
use crate::onchain::smart_farm::{Gauge, UpdatedGauges};
use bloom_offchain::execution_engine::bundled::Bundled;
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use futures::{Stream, StreamExt};
use rand::seq::index;
use serde::de::DeserializeOwned;
use serde::Serialize;
use spectrum_offchain::domain::event::{Confirmed, Traced};
use spectrum_offchain::domain::EntitySnapshot;
use std::fmt::{Debug, Display};
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
    GaugeId: Into<TaskId>
        + Copy
        + Unpin
        + Eq
        + Hash
        + Send
        + Sync
        + Display
        + Serialize
        + DeserializeOwned
        + 'static,
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
                let conf = self.conf;
                self.block_on(process_events(queue, events, indexer, tx, conf));
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
    GaugeId: Into<TaskId> + Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
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
        } => {
            let mut commands = vec![];
            for event in events {
                match event {
                    OnChainEvent::NewHarvestRequest(harvest, output) => {
                        let harvest_id = harvest.id;
                        let task_id = harvest_id.into();
                        let order = Confirmed(Bundled(harvest, output));
                        indexer.write_confirmed_harvest_order(order).await;
                        commands.push(QueueCmd::Schedule(
                            task_id,
                            Task::new_harvesting(harvest_id),
                            StrikeTime::Ready,
                        ));
                    }
                    OnChainEvent::HarvestRequestCancelled(harvest_ids) => {
                        for id in harvest_ids {
                            indexer.write_confirmed_refund_harvest_order(id).await;
                            let task_id: TaskId = id.into();
                            commands.push(QueueCmd::Cancel(task_id));
                        }
                    }
                    OnChainEvent::BotHarvestingAction {
                        payouts,
                        buffer_wallet_update,
                    } => {
                        // Index new buffer_wallet state
                        let prev_state_id = buffer_wallet_update.consumed;
                        let (entity, bearer) = buffer_wallet_update.created;
                        let bundled = Bundled(entity, bearer);
                        let traced = Traced::new(Confirmed(bundled), prev_state_id);
                        indexer.write_confirmed(traced).await;

                        for (harvest_order, _) in payouts {
                            indexer
                                .write_confirmed_spend_harvest_order(harvest_order.id)
                                .await;
                            commands.push(QueueCmd::Done(harvest_order.id.into()));
                        }
                    }
                    OnChainEvent::BotGaugeBufferingAction {
                        drained_gauges,
                        buffer_wallet_update,
                    } => {
                        // Index new buffer_wallet state
                        let prev_state_id = buffer_wallet_update.consumed;
                        let (entity, bearer) = buffer_wallet_update.created;
                        let bundled = Bundled(entity, bearer);
                        let traced = Traced::new(Confirmed(bundled), prev_state_id);
                        indexer.write_confirmed(traced).await;

                        // Index drained gauges
                        for gauge_update in drained_gauges {
                            let prev_state_id = gauge_update.consumed;
                            let (gauge, bearer) = gauge_update.created;
                            let task_id = gauge.id.into();
                            let bundled = Bundled(gauge, bearer);
                            let traced = Traced::new(Confirmed(bundled), prev_state_id);
                            indexer.write_confirmed(traced).await;
                            commands.push(QueueCmd::Done(task_id));
                        }
                    }
                    OnChainEvent::UpdatedGauges(UpdatedGauges(updated_gauges)) => {
                        for gauge_update in updated_gauges {
                            if gauge_update.created.0.balance >= conf.buffering_threshold {
                                let gauge_id = gauge_update.created.0.id;
                                commands.push(QueueCmd::Schedule(
                                    gauge_id.into(),
                                    Task::new_gauge_buffering(gauge_id),
                                    StrikeTime::Ready,
                                ));
                            }
                            let prev_state_id = gauge_update.consumed;
                            let (entity, bearer) = gauge_update.created;
                            let bundled = Bundled(entity, bearer);
                            let traced = Traced::new(Confirmed(bundled), prev_state_id);
                            indexer.write_confirmed(traced).await;
                        }
                    }
                    OnChainEvent::AuthManagerUpdated(auth_update) => {
                        let prev_state_id = auth_update.consumed;
                        let (entity, bearer) = auth_update.created;
                        let bundled = Bundled(entity, bearer);
                        let traced = Traced::new(Confirmed(bundled), prev_state_id);
                        indexer.write_confirmed(traced).await;
                    }
                }
            }

            commands.push(QueueCmd::AdvanceClocks(block_slot));
            commands
        }
        BlockEvents::RollBackward {
            events, block_slot, ..
        } => {
            let mut commands = vec![];
            for event in events {
                match event {
                    OnChainEvent::NewHarvestRequest(harvest, output) => {
                        indexer.remove_created_harvest_order(harvest.id).await;
                        commands.push(QueueCmd::Cancel(harvest.id.into()));
                    }
                    OnChainEvent::HarvestRequestCancelled(harvest_ids) => {
                        for harvest_id in harvest_ids {
                            indexer.unconsume_harvest_order(harvest_id).await;
                            commands.push(QueueCmd::Schedule(
                                harvest_id.into(),
                                Task::new_harvesting(harvest_id),
                                StrikeTime::Ready,
                            ));
                        }
                    }

                    OnChainEvent::BotHarvestingAction {
                        payouts,
                        buffer_wallet_update,
                    } => {
                        let prev_state_id = indexer
                            .remove::<BufferWallet<_>>(
                                BufferWalletId,
                                buffer_wallet_update.created.0.state_id,
                            )
                            .await;
                        assert_eq!(buffer_wallet_update.consumed, prev_state_id);
                        for (harvest_order, _) in payouts {
                            indexer.unconsume_harvest_order(harvest_order.id).await;
                            commands.push(QueueCmd::Schedule(
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
                            .await;
                        assert_eq!(buffer_wallet_update.consumed, prev_state_id);

                        for gauge_update in drained_gauges {
                            let gauge_id = gauge_update.created.0.id;
                            commands.push(QueueCmd::Schedule(
                                gauge_id.into(),
                                Task::new_gauge_buffering(gauge_id),
                                StrikeTime::Ready,
                            ));
                            let prev_state_id = indexer
                                .remove::<Gauge<_, _>>(
                                    gauge_update.created.0.id,
                                    gauge_update.created.0.state_id,
                                )
                                .await;
                            assert_eq!(gauge_update.consumed, prev_state_id);
                        }
                    }

                    OnChainEvent::UpdatedGauges(UpdatedGauges(updated_gauges)) => {
                        for gauge_update in updated_gauges {
                            if gauge_update.created.0.balance >= conf.buffering_threshold {
                                let task_id = gauge_update.created.0.id.into();
                                commands.push(QueueCmd::Cancel(task_id));
                            }
                            let prev_state_id = indexer
                                .remove::<Gauge<_, _>>(
                                    gauge_update.created.0.id,
                                    gauge_update.created.0.state_id,
                                )
                                .await;
                            assert_eq!(gauge_update.consumed, prev_state_id);
                        }
                    }
                    OnChainEvent::AuthManagerUpdated(auth_update) => {
                        let prev_state_id = indexer
                            .remove::<AuthManager<GaugeId, StateId>>(
                                AuthManagerId,
                                auth_update.created.0.state_id,
                            )
                            .await;
                        assert_eq!(auth_update.consumed, prev_state_id);
                    }
                }
            }
            commands.push(QueueCmd::DowngradeClocks(block_slot));
            commands
        }
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
