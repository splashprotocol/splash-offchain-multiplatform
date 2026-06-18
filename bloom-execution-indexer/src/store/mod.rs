use crate::domain::{BatcherDiscoverySource, BatcherProfile, ExecutionRecord, ObservedOrder, OrderRef};
use async_trait::async_trait;
use spectrum_offchain::tracing::Tracing;

pub mod rocks;

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSnapshot {
    pub orders: Vec<ObservedOrder>,
    pub executions: Vec<ExecutionRecord>,
    pub batchers: Vec<BatcherProfile>,
    pub last_slot: Option<u64>,
    pub last_block: Option<u64>,
}

#[async_trait]
pub trait ExecutionIndex {
    async fn snapshot(&self) -> IndexSnapshot;
    async fn put_snapshot(&self, snapshot: IndexSnapshot);

    async fn upsert_order(&self, order: ObservedOrder) {
        let mut snapshot = self.snapshot().await;
        if let Some(existing) = snapshot
            .orders
            .iter_mut()
            .find(|candidate| candidate.order_ref == order.order_ref)
        {
            *existing = order;
        } else {
            snapshot.orders.push(order);
        }
        self.put_snapshot(snapshot).await;
    }

    async fn upsert_execution(&self, execution: ExecutionRecord) {
        let mut snapshot = self.snapshot().await;
        if let Some(existing) = snapshot
            .executions
            .iter_mut()
            .find(|candidate| candidate.tx_hash == execution.tx_hash)
        {
            *existing = execution;
        } else {
            snapshot.executions.push(execution);
        }
        self.put_snapshot(snapshot).await;
    }

    async fn mark_order_spent(&self, order_ref: &OrderRef, status: crate::domain::OrderStatus) {
        let mut snapshot = self.snapshot().await;
        if let Some(order) = snapshot
            .orders
            .iter_mut()
            .find(|candidate| &candidate.order_ref == order_ref)
        {
            order.status = status;
        }
        self.put_snapshot(snapshot).await;
    }

    async fn upsert_batcher(&self, batcher: BatcherProfile) {
        let mut snapshot = self.snapshot().await;
        if let Some(existing) = snapshot
            .batchers
            .iter_mut()
            .find(|candidate| candidate.pkh == batcher.pkh)
        {
            if batcher.first_seen_slot < existing.first_seen_slot {
                existing.first_seen_slot = batcher.first_seen_slot;
                existing.first_seen_ms = batcher.first_seen_ms;
            }
            if batcher.last_seen_slot > existing.last_seen_slot {
                existing.last_seen_slot = batcher.last_seen_slot;
                existing.last_seen_ms = batcher.last_seen_ms;
            }
            if existing.source != batcher.source {
                existing.source = crate::domain::BatcherDiscoverySource::Both;
            }
        } else {
            snapshot.batchers.push(batcher);
        }
        self.put_snapshot(snapshot).await;
    }

    async fn set_tip(&self, slot: u64, block: u64) {
        let mut snapshot = self.snapshot().await;
        snapshot.last_slot = Some(slot);
        snapshot.last_block = Some(block);
        self.put_snapshot(snapshot).await;
    }

    async fn unapply_tx(&self, tx_hash: &str) {
        let mut snapshot = self.snapshot().await;
        snapshot.orders.retain(|order| order.order_ref.tx_hash != tx_hash);
        snapshot
            .executions
            .retain(|execution| execution.tx_hash != tx_hash);
        for order in &mut snapshot.orders {
            match &order.status {
                crate::domain::OrderStatus::Executed { execution_tx, .. } if execution_tx == tx_hash => {
                    order.status = crate::domain::OrderStatus::Open;
                }
                crate::domain::OrderStatus::Cancelled { tx, .. }
                | crate::domain::OrderStatus::UnknownSpent { tx, .. }
                    if tx == tx_hash =>
                {
                    order.status = crate::domain::OrderStatus::Open;
                }
                _ => {}
            }
        }
        snapshot.batchers = rebuild_batchers(&snapshot);
        self.put_snapshot(snapshot).await;
    }
}

fn rebuild_batchers(snapshot: &IndexSnapshot) -> Vec<BatcherProfile> {
    let mut batchers = Vec::new();

    for order in &snapshot.orders {
        for pkh in &order.permitted_executors {
            merge_batcher(
                &mut batchers,
                BatcherProfile {
                    pkh: pkh.clone(),
                    first_seen_slot: order.created_slot,
                    first_seen_ms: order.created_time_ms,
                    last_seen_slot: order.created_slot,
                    last_seen_ms: order.created_time_ms,
                    source: BatcherDiscoverySource::PermittedExecutor,
                },
            );
        }
    }

    for execution in &snapshot.executions {
        for pkh in &execution.signer_batchers {
            merge_batcher(
                &mut batchers,
                BatcherProfile {
                    pkh: pkh.clone(),
                    first_seen_slot: execution.slot,
                    first_seen_ms: execution.time_ms,
                    last_seen_slot: execution.slot,
                    last_seen_ms: execution.time_ms,
                    source: BatcherDiscoverySource::ExecutionSigner,
                },
            );
        }
    }

    batchers.sort_by(|left, right| left.pkh.cmp(&right.pkh));
    batchers
}

fn merge_batcher(batchers: &mut Vec<BatcherProfile>, batcher: BatcherProfile) {
    if let Some(existing) = batchers.iter_mut().find(|candidate| candidate.pkh == batcher.pkh) {
        if batcher.first_seen_slot < existing.first_seen_slot {
            existing.first_seen_slot = batcher.first_seen_slot;
            existing.first_seen_ms = batcher.first_seen_ms;
        }
        if batcher.last_seen_slot > existing.last_seen_slot {
            existing.last_seen_slot = batcher.last_seen_slot;
            existing.last_seen_ms = batcher.last_seen_ms;
        }
        if existing.source != batcher.source {
            existing.source = BatcherDiscoverySource::Both;
        }
    } else {
        batchers.push(batcher);
    }
}

#[async_trait]
impl<Index> ExecutionIndex for Tracing<Index>
where
    Index: ExecutionIndex + Send + Sync,
{
    async fn snapshot(&self) -> IndexSnapshot {
        self.component.snapshot().await
    }

    async fn put_snapshot(&self, snapshot: IndexSnapshot) {
        self.component.put_snapshot(snapshot).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ExecutionAttribution, ExecutionClassification, OrderRef};

    #[test]
    fn rebuild_batchers_keeps_permissionless_execution_signers() {
        let signer = "abc".to_string();
        let snapshot = IndexSnapshot {
            executions: vec![ExecutionRecord {
                tx_hash: "tx".to_string(),
                slot: 42,
                time_ms: Some(1000),
                attribution: ExecutionAttribution::UnknownPermissionless,
                signer_batchers: vec![signer.clone()],
                consumed_orders: vec![OrderRef {
                    tx_hash: "order".to_string(),
                    output_index: 0,
                }],
                pair: None,
                classification: ExecutionClassification::UnknownSpent,
            }],
            ..Default::default()
        };

        let batchers = rebuild_batchers(&snapshot);

        assert_eq!(batchers.len(), 1);
        assert_eq!(batchers[0].pkh, signer);
        assert_eq!(batchers[0].source, BatcherDiscoverySource::ExecutionSigner);
    }
}
