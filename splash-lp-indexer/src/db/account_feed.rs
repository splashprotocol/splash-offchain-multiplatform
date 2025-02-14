use crate::db::{read_max_key, ACCOUNT_FEED_CF};
use crate::onchain::event::AccountEvent;
use async_trait::async_trait;
use rocksdb::TransactionDB;
use std::sync::Arc;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait AccountEventFeed {
    async fn batch_append(&self, events: Vec<AccountEvent>);
}

#[async_trait]
impl AccountEventFeed for Arc<TransactionDB> {
    async fn batch_append(&self, events: Vec<AccountEvent>) {
        let db = self.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let account_feed_cf = db.cf_handle(ACCOUNT_FEED_CF).unwrap();
            let mut seq_num = read_max_key(&tx, account_feed_cf);
            for event in events {
                seq_num += 1;
                let event_key = rmp_serde::to_vec(&seq_num).unwrap();
                let event_value = rmp_serde::to_vec_named(&event).unwrap();
                tx.put_cf(account_feed_cf, event_key, event_value).unwrap();
            }
        })
        .await
        .unwrap()
    }
}
