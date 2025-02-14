use crate::db::event_log::EventLog;
use crate::onchain::event::OnChainEvent;
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use futures::Stream;
use futures::StreamExt;

pub async fn log_lp_events<U, Log>(upstream: U, log: &Log)
where
    U: Stream<Item = (BlockEvents<OnChainEvent>, TransactionHandle)>,
    Log: EventLog,
{
    upstream
        .for_each(|(block, transaction_handle)| async move {
            log_event(block, log).await;
            transaction_handle.commit();
        })
        .await
}

async fn log_event<Log>(events: BlockEvents<OnChainEvent>, log: &Log)
where
    Log: EventLog,
{
    match events {
        BlockEvents::RollForward {
            events,
            slot: block_num,
            ..
        } => log.batch_append(block_num, events).await,
        BlockEvents::RollBackward {
            events,
            slot: block_num,
            ..
        } => log.batch_discard(block_num, events).await,
    }
}
