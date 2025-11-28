use crate::onchain::event::OnChainEvent;
use crate::position_db::{
    event_key, rollback_suspended_pools, set_suspended_pools, ColumnFamilies, PositionDB, CURRENT_SLOT_KEY,
};
use async_trait::async_trait;
use cml_core::Slot;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait EventLog {
    async fn batch_append(&self, block_slot: Slot, events: Vec<OnChainEvent>);
    async fn batch_discard(&self, block_slot: Slot, events: Vec<OnChainEvent>);
}

#[async_trait]
impl EventLog for PositionDB {
    async fn batch_append(&self, block_slot: Slot, events: Vec<OnChainEvent>) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let cfs = ColumnFamilies::new(&db);
            let tx = db.transaction();
            tx.put_cf(&cfs.kv, CURRENT_SLOT_KEY, rmp_serde::to_vec(&block_slot).unwrap())
                .unwrap();
            for (n, event) in events.iter().enumerate() {
                if let OnChainEvent::PermManagerUpdate(update) = event {
                    set_suspended_pools(&tx, cfs.suspended_pools, update, block_slot);
                } else {
                    let key = event_key(block_slot, n);
                    tx.put_cf(cfs.events, key, rmp_serde::to_vec_named(&event).unwrap())
                        .unwrap();
                }
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn batch_discard(&self, block_slot: Slot, events: Vec<OnChainEvent>) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let cfs = ColumnFamilies::new(&db);
            let tx = db.transaction();
            for (n, event) in events.iter().enumerate() {
                if let OnChainEvent::PermManagerUpdate(update) = event {
                    rollback_suspended_pools(&tx, cfs.suspended_pools, update, block_slot);
                } else {
                    let key = event_key(block_slot, n);
                    tx.delete_cf(cfs.events, key).unwrap();
                }
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }
}
