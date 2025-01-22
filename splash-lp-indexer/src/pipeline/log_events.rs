use crate::event::LpEvent;
use crate::event_log::EventLog;
use cardano_chain_sync::atomic_flow::TransactionHandle;
use cardano_chain_sync::data::LedgerBlockEvent;
use futures::Stream;
use futures::StreamExt;

pub async fn log_lp_events<U, Log>(upstream: U, log: &Log)
where
    U: Stream<Item = (LedgerBlockEvent<Vec<LpEvent>>, TransactionHandle)>,
    Log: EventLog,
{
    upstream
        .for_each(|(ledger_block, transaction_handle)| async move {
            log_event(ledger_block, log).await;
            transaction_handle.commit();
        })
        .await
}

async fn log_event<Log>(event: LedgerBlockEvent<Vec<LpEvent>>, log: &Log)
where
    Log: EventLog,
{
    match event {
        LedgerBlockEvent::RollForward(events) => log.batch_append(events).await,
        LedgerBlockEvent::RollBackward(events) => log.batch_discard(events).await,
    }
}
