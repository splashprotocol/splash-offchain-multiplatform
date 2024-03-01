use std::time::Duration;

#[async_trait::async_trait]
pub trait TransactionTracker<TxId> {
    async fn track_status<S, F>(&self, tx_id: TxId, give_up_after: Duration, on_success: S, on_failure: F)
    where
        S: FnOnce(TxId),
        F: FnOnce(TxId);
}
