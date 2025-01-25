use crate::db::event_log::EventLog;
use crate::event::LpEvent;
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use futures::Stream;
use futures::StreamExt;

pub async fn log_lp_events<U, Log>(upstream: U, log: &Log)
where
    U: Stream<Item = (BlockEvents<LpEvent>, TransactionHandle)>,
    Log: EventLog,
{
    upstream
        .for_each(|(block, transaction_handle)| async move {
            log_event(block, log).await;
            transaction_handle.commit();
        })
        .await
}

async fn log_event<Log>(events: BlockEvents<LpEvent>, log: &Log)
where
    Log: EventLog,
{
    match events {
        BlockEvents::RollForward { events, block_num } => log.batch_append(block_num, events).await,
        BlockEvents::RollBackward { events, block_num } => log.batch_discard(block_num, events).await,
    }
}
