use crate::feed::event::ExportAccountEvent;
use crate::position_db::{read_max_key, read_min_kv, PositionDB, ACCOUNT_FEED_CF};
use async_trait::async_trait;
use rocksdb::{Transaction, TransactionDB};
use tokio::task::spawn_blocking;

pub(crate) fn batch_append(
    tx: &Transaction<TransactionDB>,
    events: Vec<ExportAccountEvent>,
    cf: &rocksdb::ColumnFamily,
) {
    let mut seq_num = read_max_key(&tx, cf);
    for event in events {
        seq_num += 1;
        let event_key = rmp_serde::to_vec(&seq_num).unwrap();
        let event_value = rmp_serde::to_vec_named(&event).unwrap();
        tx.put_cf(cf, event_key, event_value).unwrap();
    }
}

#[async_trait]
pub trait ExportEventFeed {
    async fn next(&self) -> Option<(u64, ExportAccountEvent)>;
    async fn delete(&self, seq_num: u64);
}

#[async_trait]
impl ExportEventFeed for PositionDB {
    async fn next(&self) -> Option<(u64, ExportAccountEvent)> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let account_feed_cf = db.cf_handle(ACCOUNT_FEED_CF).unwrap();
            read_min_kv(&db, account_feed_cf)
        })
        .await
        .unwrap()
    }

    async fn delete(&self, seq_num: u64) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let account_feed_cf = db.cf_handle(ACCOUNT_FEED_CF).unwrap();
            let event_key = rmp_serde::to_vec(&seq_num).unwrap();
            db.delete_cf(account_feed_cf, &event_key).unwrap();
        })
        .await
        .unwrap()
    }
}
