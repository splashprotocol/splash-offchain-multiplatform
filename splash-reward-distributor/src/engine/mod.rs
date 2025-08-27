mod batch;
pub mod executor;
mod prover;
pub mod queue;
pub mod resolved_tx;
mod task;
pub mod verifier;
mod withdrawal;

use crate::engine::executor::{BatchExecutor, Control, Error as ExecutorError};
use crate::engine::queue::{QueueCmd, StrikeTime, TaskQueue};
use crate::engine::task::{Task, TaskId};
use crate::entity_index::{AuthManagerIndex, BufferWalletIndex, GaugeIndex, HarvestOrderIndex};
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use cml_chain::transaction::Transaction;
use cml_crypto::TransactionHash;
use futures::channel::mpsc::{Receiver, Sender};
use futures::{FutureExt, SinkExt, Stream, StreamExt};
use serde::Deserialize;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::tx_view::TimedOutput;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::persistent_index::PersistentIndex;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::onchain::funding_box::FundingBoxSnapshot;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::funding::FundingRepo;
use splash_dao_offchain::protocol_config::{
    BufferWalletScript, OperatorCreds, PermManagerAuthPolicy, SplashPolicy,
};
use splash_dao_offchain::routines::{Slot, TimedOutputRef};
use splash_yf_offchain::entities::buffer_wallet::try_extract_buffer_wallet;
use splash_yf_offchain::entities::harvest_order::try_extract_harvest_order;
use splash_yf_offchain::entities::smart_farm::{try_extract_gauge, UpdatedGauges};
use splash_yf_offchain::events::OnChainEvent;
use splash_yf_offchain::settings::MinLovelacePerHarvest;
use std::fmt::Debug;
use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::task::{Context, Poll};
use type_equalities::IsEqual;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct EngineConfig {
    buffering_threshold: u64,
}

pub struct Engine<U, Q, E> {
    event_stream: U,
    queue: Q,
    executor: E,
    current_task: Option<Pin<Box<dyn Future<Output = ControlFlow<(), ()>> + Send>>>,
    dropped_unconfirmed_tx_hashes_recv: Receiver<TransactionHash>,
    conf: EngineConfig,
}

impl<U, Q, E> Engine<U, Q, E> {
    pub fn new(
        event_stream: U,
        queue: Q,
        executor: E,
        conf: EngineConfig,
        dropped_unconfirmed_tx_hashes_recv: Receiver<TransactionHash>,
    ) -> Self {
        Self {
            event_stream,
            queue,
            executor,
            current_task: None,
            conf,
            dropped_unconfirmed_tx_hashes_recv,
        }
    }

    fn block_on(&mut self, task: impl Future<Output = ControlFlow<(), ()>> + Send + 'static) {
        self.current_task = Some(Box::pin(task));
    }
}

impl<GaugeId, StateId, Bearer, U, Q, E> Future for Engine<U, Q, E>
where
    GaugeId: Copy + Into<TaskId> + Unpin + Send + 'static,
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

            let dropped_tx_hash = if let Poll::Ready(tx_hash) =
                Stream::poll_next(Pin::new(&mut self.dropped_unconfirmed_tx_hashes_recv), cx)
            {
                tx_hash
            } else {
                None
            };
            let executor = self.executor.clone();
            self.block_on(process_tasks(queue, executor, dropped_tx_hash));
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
                        .into_iter()
                        .map(|gauge_update| {
                            let task_id = gauge_update.created.0.id.into();
                            QueueCmd::Done(task_id, tx_hash)
                        })
                        .chain(std::iter::once(QueueCmd::ConfirmTx(tx_hash, Slot(block_slot))))
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
                OnChainEvent::AuthManagerUpdated(_) | OnChainEvent::Funding { .. } => None,
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
                OnChainEvent::AuthManagerUpdated(_) | OnChainEvent::Funding { .. } => None,
            })
            .flatten()
            .chain(vec![QueueCmd::DowngradeClocks(block_slot)])
            .collect(),
    };
    queue.clone().batch_execute(commands).await;
    tx.commit();
    ControlFlow::Continue(())
}

async fn process_tasks<GaugeId, StateId, Q, E>(
    queue: Q,
    mut executor: E,
    dropped_tx_hash: Option<TransactionHash>,
) -> ControlFlow<(), ()>
where
    Q: TaskQueue<TaskId, Task<GaugeId, StateId>> + Clone,
    E: BatchExecutor<TaskId, Task<GaugeId, StateId>, TransactionHash, ExecutorError>,
{
    let mut invalid_tasks = vec![];
    let mut stream = queue.clone().pending_stream();
    if let Some(tx_hash) = dropped_tx_hash {
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
            queue.clone().batch_execute(cmds).await;
        }
    }
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
    match executor.execute().await {
        Ok(res) => {
            let tx_hash = res.output;

            let commands = res
                .executed_tasks
                .into_iter()
                .map(|task_id| QueueCmd::Done(task_id, tx_hash))
                .chain(invalid_tasks.into_iter().map(QueueCmd::Cancel));

            queue.batch_execute(commands.collect()).await;
        }
        Err(ExecutorError::TxInputsAlreadySpent { failed_task_ids }) => {
            let commands = failed_task_ids.into_iter().map(QueueCmd::Cancel).collect();
            queue.batch_execute(commands).await;
        }
        Err(_) => (),
    }
    ControlFlow::Continue(())
}

pub async fn update_index_from_mempool_dropped_tx<OnChainIndex, Utxos, Ctx, FB>(
    recv: Receiver<Transaction>,
    send: Sender<TransactionHash>,
    index: OnChainIndex,
    funding: FB,
    utxos: Utxos,
    ctx: Ctx,
) where
    OnChainIndex: HarvestOrderIndex<OutputRef, FinalizedTxOut>
        + BufferWalletIndex<OutputRef, FinalizedTxOut>
        + GaugeIndex<FarmId, OutputRef, FinalizedTxOut>
        + AuthManagerIndex<FarmId, OutputRef, FinalizedTxOut>
        + Clone,
    Utxos: PersistentIndex<OutputRef, TimedOutput> + Clone,
    Ctx: Has<PermManagerAuthPolicy>
        + Has<BufferWalletScript>
        + Has<OperatorCreds>
        + Has<SplashPolicy>
        + Has<MinLovelacePerHarvest>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>,
    FB: FundingRepo + Send + Sync,
{
    recv.for_each(|tx| async {
        let tx_hash = tx.body.hash();

        send.clone().send(tx_hash).await.ok();

        for input in tx.body.inputs {
            let output_ref = OutputRef::from(input);
            let funding_ctx = FundingCtx {
                creds: ctx.select::<OperatorCreds>(),
                output_ref,
            };
            if let Some(TimedOutput { output, .. }) = utxos.get(output_ref).await {
                // We don't need the actual slot value to parse the following entity; a dummy value suffices
                if try_extract_harvest_order(&output, output_ref, Slot(100), &ctx).is_some() {
                    index.unconsume_harvest_order(output_ref).await;
                } else if FundingBoxSnapshot::try_from_ledger(&output, &funding_ctx).is_some() {
                    funding.unspend_predicted(output_ref.into()).await;
                }
            }
        }

        for (ix, output) in tx.body.outputs.into_iter().enumerate() {
            let output_ref = OutputRef::new(tx_hash, ix as u64);
            // Again, it's fine to have a dummy slot value
            let timed_output_ref = TimedOutputRef::new(output_ref, Slot(100));

            let funding_ctx = FundingCtx {
                creds: ctx.select::<OperatorCreds>(),
                output_ref,
            };
            if let Some(gauge) = try_extract_gauge(&output, timed_output_ref, &ctx) {
                index.remove_gauge(gauge.id, output_ref).await;
            } else if try_extract_buffer_wallet(&output, output_ref, &ctx).is_some() {
                index.remove_buffer_wallet(output_ref).await;
            } else if FundingBoxSnapshot::try_from_ledger(&output, &funding_ctx).is_some() {
                funding.eliminate_predicted(output_ref.into()).await;
            }
        }
    })
    .await;
}

struct FundingCtx {
    creds: OperatorCreds,
    output_ref: OutputRef,
}

impl Has<OutputRef> for FundingCtx {
    fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.output_ref
    }
}

impl Has<OperatorCreds> for FundingCtx {
    fn select<U: IsEqual<OperatorCreds>>(&self) -> OperatorCreds {
        self.creds.clone()
    }
}
