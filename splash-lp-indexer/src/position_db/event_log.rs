use crate::account::DefaultEpochSlotConversion;
use crate::onchain::event::OnChainEvent;
use crate::position_db::{
    event_key, gauge_key, get_account_positions_rollback_to_slot, rollback_account_positions,
    rollback_active_pools, rollback_suspended_pools, set_account_positions_rollback_to_slot,
    set_active_pools, set_suspended_pools, ColumnFamilies, PositionDB,
    ACCOUNT_POSITIONS_ROLLBACK_TO_SLOT_KEY, CURRENT_SLOT_KEY,
};
use async_trait::async_trait;
use cml_core::Slot;
use log::trace;
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
        let epoch_start = self.epoch_start;
        let num_slots_in_epoch = self.num_slots_in_epoch;
        spawn_blocking(move || {
            let cfs = ColumnFamilies::new(&db);
            let tx = db.transaction();

            if let Some(rollback_to_slot) = get_account_positions_rollback_to_slot(&tx, cfs.kv) {
                // We come to this line on the first appended block after a chain-rollback. Apply rollback
                // on the `AccountPosition`s first.
                let epoch_converter = DefaultEpochSlotConversion::new(epoch_start, num_slots_in_epoch);
                rollback_account_positions(&tx, &cfs, epoch_converter, rollback_to_slot);
                tx.delete_cf(cfs.kv, ACCOUNT_POSITIONS_ROLLBACK_TO_SLOT_KEY)
                    .unwrap();
            }

            tx.put_cf(&cfs.kv, CURRENT_SLOT_KEY, rmp_serde::to_vec(&block_slot).unwrap())
                .unwrap();
            for (n, event) in events.iter().enumerate() {
                match event {
                    OnChainEvent::PermManagerUpdate(update) => {
                        set_suspended_pools(&tx, cfs.suspended_pools, update, block_slot);
                    }
                    OnChainEvent::NewWeightingPoll(active_pools) => {
                        trace!(
                            "set_active_pools: epoch: {}, pools: {:?}",
                            active_pools.0,
                            active_pools.1
                        );
                        set_active_pools(&tx, cfs.active_pools, active_pools.0, &active_pools.1);
                    }
                    _ => {
                        let key = event_key(block_slot, n);
                        tx.put_cf(cfs.events, key, rmp_serde::to_vec_named(&event).unwrap())
                            .unwrap();
                    }
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
                match event {
                    OnChainEvent::PermManagerUpdate(update) => {
                        rollback_suspended_pools(&tx, cfs.suspended_pools, update, block_slot);
                    }
                    OnChainEvent::NewWeightingPoll(active_pools) => {
                        rollback_active_pools(&tx, cfs.active_pools, active_pools.0, &active_pools.1);
                    }
                    _ => {
                        if let OnChainEvent::Gauge(gauge) = event {
                            tx.delete_cf(cfs.gauge_weights, gauge_key(gauge.pool_id, gauge.epoch))
                                .unwrap();
                        }
                        let key = event_key(block_slot, n);
                        tx.delete_cf(cfs.events, key).unwrap();
                    }
                }
            }

            set_account_positions_rollback_to_slot(&tx, cfs.kv, block_slot);
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }
}
