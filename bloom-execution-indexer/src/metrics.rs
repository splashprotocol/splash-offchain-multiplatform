use crate::domain::{BatcherMetrics, ExecutionAttribution, ExecutionClassification, OrderStatus};
use crate::store::IndexSnapshot;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default)]
pub struct MetricsFilter {
    pub from_ms: Option<u64>,
    pub to_ms: Option<u64>,
    pub pair: Option<String>,
}

pub fn compute_batcher_metrics(snapshot: &IndexSnapshot, pkh: &str, filter: MetricsFilter) -> BatcherMetrics {
    let mut metrics = BatcherMetrics {
        batcher: pkh.to_string(),
        from_ms: filter.from_ms,
        to_ms: filter.to_ms,
        pair: filter.pair.clone(),
        ..Default::default()
    };
    let mut response_times = Vec::new();
    let mut counted_orders = BTreeSet::new();

    for order in snapshot.orders.iter().filter(|order| {
        order.permitted_executors.iter().any(|executor| executor == pkh)
            && filter
                .from_ms
                .map(|from| {
                    order
                        .created_time_ms
                        .map(|created| created >= from)
                        .unwrap_or(false)
                })
                .unwrap_or(true)
            && filter
                .to_ms
                .map(|to| {
                    order
                        .created_time_ms
                        .map(|created| created <= to)
                        .unwrap_or(false)
                })
                .unwrap_or(true)
            && filter
                .pair
                .as_ref()
                .map(|pair| &order.pair == pair)
                .unwrap_or(true)
    }) {
        counted_orders.insert(order.order_ref.clone());
        metrics.eligible_orders += 1;
        *metrics
            .eligible_input_volume_by_asset
            .entry(order.input_asset.clone())
            .or_default() += order.input_amount as u128;

        match &order.status {
            OrderStatus::Open => metrics.still_open_eligible_orders += 1,
            OrderStatus::Executed {
                attribution, time_ms, ..
            } => match attribution {
                ExecutionAttribution::Single(executor) if executor == pkh => {
                    metrics.executed_orders += 1;
                    if let (Some(executed_ms), Some(created_ms)) = (time_ms, order.created_time_ms) {
                        response_times.push(executed_ms.saturating_sub(created_ms));
                    }
                    *metrics
                        .executed_input_volume_by_asset
                        .entry(order.input_asset.clone())
                        .or_default() += order.input_amount as u128;
                    if let Some(output_amount) = order.output_amount {
                        *metrics
                            .executed_output_volume_by_asset
                            .entry(order.output_asset.clone())
                            .or_default() += output_amount as u128;
                    }
                }
                ExecutionAttribution::Ambiguous(executors) => {
                    metrics.ambiguous_executions += 1;
                    if executors.iter().any(|executor| executor == pkh) {
                        metrics.missed_eligible_orders += 1;
                    } else {
                        metrics.missed_eligible_orders += 1;
                    }
                }
                ExecutionAttribution::Unknown | ExecutionAttribution::UnknownPermissionless => {
                    metrics.unknown_executions += 1;
                    metrics.missed_eligible_orders += 1;
                }
                ExecutionAttribution::Single(_) => metrics.missed_eligible_orders += 1,
            },
            OrderStatus::Cancelled { .. } | OrderStatus::UnknownSpent { .. } => {
                metrics.missed_eligible_orders += 1;
            }
        }
    }

    for execution in snapshot.executions.iter().filter(|execution| {
        execution.signer_batchers.iter().any(|signer| signer == pkh)
            && execution.classification == ExecutionClassification::Executed
    }) {
        for order_ref in &execution.consumed_orders {
            if counted_orders.contains(order_ref) {
                continue;
            }
            let Some(order) = snapshot
                .orders
                .iter()
                .find(|candidate| &candidate.order_ref == order_ref)
            else {
                continue;
            };
            if !order.permitted_executors.is_empty() {
                continue;
            }
            if !filter
                .from_ms
                .map(|from| order.created_time_ms.map(|created| created >= from).unwrap_or(false))
                .unwrap_or(true)
            {
                continue;
            }
            if !filter
                .to_ms
                .map(|to| order.created_time_ms.map(|created| created <= to).unwrap_or(false))
                .unwrap_or(true)
            {
                continue;
            }
            if !filter
                .pair
                .as_ref()
                .map(|pair| &order.pair == pair)
                .unwrap_or(true)
            {
                continue;
            }

            counted_orders.insert(order.order_ref.clone());
            metrics.eligible_orders += 1;
            metrics.executed_orders += 1;
            *metrics
                .eligible_input_volume_by_asset
                .entry(order.input_asset.clone())
                .or_default() += order.input_amount as u128;
            *metrics
                .executed_input_volume_by_asset
                .entry(order.input_asset.clone())
                .or_default() += order.input_amount as u128;
            if let Some(output_amount) = order.output_amount {
                *metrics
                    .executed_output_volume_by_asset
                    .entry(order.output_asset.clone())
                    .or_default() += output_amount as u128;
            }
            if let (Some(executed_ms), Some(created_ms)) = (execution.time_ms, order.created_time_ms) {
                response_times.push(executed_ms.saturating_sub(created_ms));
            }
        }
    }

    metrics.capture_rate = if metrics.eligible_orders == 0 {
        None
    } else {
        Some(format!(
            "{:.4}",
            metrics.executed_orders as f64 / metrics.eligible_orders as f64
        ))
    };
    metrics.median_response_ms = percentile(&mut response_times.clone(), 50);
    metrics.p95_response_ms = percentile(&mut response_times, 95);
    metrics
}

fn percentile(values: &mut [u64], percentile: u64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let rank = ((values.len() as u64 * percentile).saturating_add(99) / 100).saturating_sub(1);
    values.get(rank as usize).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ExecutionAttribution, ObservedOrder, OrderKind, OrderRef, OrderStatus};

    fn order(index: u64, pkh: &str, status: OrderStatus) -> ObservedOrder {
        ObservedOrder {
            order_ref: OrderRef {
                tx_hash: format!("tx{index}"),
                output_index: 0,
            },
            kind: OrderKind::Limit,
            pair: "a/b".to_string(),
            input_asset: "a".to_string(),
            output_asset: "b".to_string(),
            input_amount: 100,
            output_amount: Some(50),
            price_num: 1,
            price_denom: 1,
            permitted_executors: vec![pkh.to_string()],
            created_slot: index,
            created_time_ms: Some(index * 1000),
            status,
        }
    }

    #[test]
    fn eligible_orders_partition_into_executed_open_and_missed() {
        let pkh = "aa";
        let snapshot = IndexSnapshot {
            orders: vec![
                order(
                    1,
                    pkh,
                    OrderStatus::Executed {
                        execution_tx: "e1".to_string(),
                        attribution: ExecutionAttribution::Single(pkh.to_string()),
                        slot: 2,
                        time_ms: Some(3_000),
                    },
                ),
                order(2, pkh, OrderStatus::Open),
                order(
                    3,
                    pkh,
                    OrderStatus::Executed {
                        execution_tx: "e2".to_string(),
                        attribution: ExecutionAttribution::Unknown,
                        slot: 4,
                        time_ms: Some(4_000),
                    },
                ),
            ],
            ..Default::default()
        };

        let metrics = compute_batcher_metrics(&snapshot, pkh, MetricsFilter::default());

        assert_eq!(metrics.eligible_orders, 3);
        assert_eq!(metrics.executed_orders, 1);
        assert_eq!(metrics.still_open_eligible_orders, 1);
        assert_eq!(metrics.missed_eligible_orders, 1);
        assert_eq!(
            metrics.eligible_orders,
            metrics.executed_orders + metrics.still_open_eligible_orders + metrics.missed_eligible_orders
        );
        assert_eq!(metrics.median_response_ms, Some(2_000));
    }

    #[test]
    fn permissionless_execution_signer_counts_as_executed_work() {
        let pkh = "aa";
        let mut observed = order(
            1,
            "",
            OrderStatus::Executed {
                execution_tx: "e1".to_string(),
                attribution: ExecutionAttribution::UnknownPermissionless,
                slot: 2,
                time_ms: Some(3_000),
            },
        );
        observed.permitted_executors = vec![];
        let snapshot = IndexSnapshot {
            orders: vec![observed],
            executions: vec![crate::domain::ExecutionRecord {
                tx_hash: "e1".to_string(),
                slot: 2,
                time_ms: Some(3_000),
                attribution: ExecutionAttribution::UnknownPermissionless,
                signer_batchers: vec![pkh.to_string()],
                consumed_orders: vec![crate::domain::OrderRef {
                    tx_hash: "tx1".to_string(),
                    output_index: 0,
                }],
                pair: Some("a/b".to_string()),
                classification: ExecutionClassification::Executed,
            }],
            ..Default::default()
        };

        let metrics = compute_batcher_metrics(&snapshot, pkh, MetricsFilter::default());

        assert_eq!(metrics.eligible_orders, 1);
        assert_eq!(metrics.executed_orders, 1);
        assert_eq!(metrics.still_open_eligible_orders, 0);
        assert_eq!(metrics.missed_eligible_orders, 0);
        assert_eq!(metrics.median_response_ms, Some(2_000));
    }
}
