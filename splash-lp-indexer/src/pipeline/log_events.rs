use crate::onchain::event::OnChainEvent;
use crate::position_db::event_log::EventLog;
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use futures::Stream;
use futures::StreamExt;
use log::trace;

pub async fn log_onchain_events<U, Log>(upstream: U, log: &Log)
where
    U: Stream<Item = (BlockEvents<OnChainEvent>, Option<TransactionHandle>)>,
    Log: EventLog,
{
    upstream
        .for_each(|(block, transaction_handle)| async move {
            log_event(block, log).await;
            if let Some(transaction_handle) = transaction_handle {
                transaction_handle.commit();
            }
        })
        .await
}

pub async fn log_event<Log>(events: BlockEvents<OnChainEvent>, log: &Log)
where
    Log: EventLog,
{
    match events {
        BlockEvents::RollForward {
            events, block_slot, ..
        } => {
            trace!(
                "log_event: roll_forward slot: {}, events: {:?}",
                block_slot,
                events
            );
            log.batch_append(block_slot, events).await
        }
        BlockEvents::RollBackward {
            events, block_slot, ..
        } => log.batch_discard(block_slot, events).await,
    }
}
