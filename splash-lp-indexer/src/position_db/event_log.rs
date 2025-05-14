use std::fmt::Debug;
use crate::onchain::event::{OnChainEvent, WithOptionalSlot};
use crate::position_db::{event_key, PositionDB, AGGREGATE_CF, EVENTS_CF, MAX_BLOCK_NUM_KEY};
use async_trait::async_trait;
use cml_core::Slot;
use log::info;
use serde::Serialize;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait EventLog<A> {
    async fn batch_append(&self, block_slot: u64, events: Vec<A>);
    async fn batch_discard(&self, block_slot: u64, events: Vec<A>);
}

#[async_trait]
impl<A> EventLog<A> for PositionDB where A: Serialize + Send + Sync + Debug + WithOptionalSlot + Clone + 'static {
    async fn batch_append(&self, block_slot: u64, events: Vec<A>) {
        info!("Going to add events for block at slot {}. Events qty: {}, Events are: {}", block_slot, events.len(), events.iter().map(|e| format!("{:?}", e)).collect::<Vec<_>>().join(", "));
        let db = self.db.clone();
        spawn_blocking(move || {
            let events_cf = db.cf_handle(EVENTS_CF).unwrap();
            let aggregates_cf = db.cf_handle(AGGREGATE_CF).unwrap();
            let tx = db.transaction();
            tx.put_cf(
                aggregates_cf,
                MAX_BLOCK_NUM_KEY,
                &rmp_serde::to_vec(&block_slot).unwrap(),
            )
            .unwrap();
            for (n, event) in events.iter().enumerate() {
                let slot = event.slot().map(|sl| sl.0).unwrap_or(block_slot);
                let key = event_key(slot, n);
                tx.put_cf(events_cf, key, rmp_serde::to_vec_named(&event).unwrap())
                    .unwrap();
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn batch_discard(&self, block_slot: u64, events: Vec<A>) {
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
