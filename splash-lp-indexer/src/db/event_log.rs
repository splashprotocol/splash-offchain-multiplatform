use crate::db::{RocksDB, EVENTS_CF};
use crate::event::LpEvent;
use async_trait::async_trait;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait EventLog {
    async fn batch_append(&self, block_num: u64, events: Vec<LpEvent>);
    async fn batch_discard(&self, block_num: u64, events: Vec<LpEvent>);
}

#[async_trait]
impl EventLog for RocksDB {
    async fn batch_append(&self, block_num: u64, events: Vec<LpEvent>) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let events_cf = db.cf_handle(EVENTS_CF).unwrap();
            let tx = db.transaction();
            for (n, event) in events.iter().enumerate() {
                let key = full_key(block_num, n);
                tx.put_cf(events_cf, key, rmp_serde::to_vec_named(&event).unwrap())
                    .unwrap();
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn batch_discard(&self, block_num: u64, events: Vec<LpEvent>) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let events_cf = db.cf_handle(EVENTS_CF).unwrap();
            let tx = db.transaction();
            for (n, _) in events.iter().enumerate() {
                let key = full_key(block_num, n);
                tx.delete_cf(events_cf, key).unwrap();
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }
}

fn full_key(block_num: u64, event_num: usize) -> Vec<u8> {
    let mut bf = vec![];
    bf.extend_from_slice(block_num.to_be_bytes().as_ref());
    bf.extend_from_slice(event_num.to_be_bytes().as_ref());
    bf
}

fn block_key(block_num: u64) -> Vec<u8> {
    block_num.to_be_bytes().to_vec()
}
