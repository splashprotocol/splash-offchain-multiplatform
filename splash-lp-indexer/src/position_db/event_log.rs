use crate::onchain::event::OnChainEvent;
use crate::position_db::{event_key, PositionDB, KV_CF, EVENTS_CF, MAX_SLOT_KEY};
use async_trait::async_trait;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait EventLog {
    async fn batch_append(&self, block_slot: u64, events: Vec<OnChainEvent>);
    async fn batch_discard(&self, block_slot: u64, events: Vec<OnChainEvent>);
}

#[async_trait]
impl EventLog for PositionDB {
    async fn batch_append(&self, block_slot: u64, events: Vec<OnChainEvent>) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let events_cf = db.cf_handle(EVENTS_CF).unwrap();
            let aggregates_cf = db.cf_handle(KV_CF).unwrap();
            let tx = db.transaction();
            tx.put_cf(
                aggregates_cf,
                MAX_SLOT_KEY,
                &rmp_serde::to_vec(&block_slot).unwrap(),
            )
            .unwrap();
            for (n, event) in events.iter().enumerate() {
                let key = event_key(block_slot, n);
                tx.put_cf(events_cf, key, rmp_serde::to_vec_named(&event).unwrap())
                    .unwrap();
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn batch_discard(&self, block_slot: u64, events: Vec<OnChainEvent>) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let events_cf = db.cf_handle(EVENTS_CF).unwrap();
            let tx = db.transaction();
            for (n, _) in events.iter().enumerate() {
                let key = event_key(block_slot, n);
                tx.delete_cf(events_cf, key).unwrap();
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }
}
