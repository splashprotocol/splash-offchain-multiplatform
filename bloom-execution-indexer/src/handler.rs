use crate::domain::{
    pair_id, BatcherDiscoverySource, BatcherProfile, ExecutionAttribution, ExecutionClassification,
    ExecutionRecord, ObservedOrder, OrderKind, OrderRef, OrderStatus,
};
use crate::store::ExecutionIndex;
use async_trait::async_trait;
use bloom_offchain_cardano::event_sink::tx_view::TxViewMut;
use bloom_offchain_cardano::orders::limit::LimitOrderObservation;
use cardano_chain_sync::data::LedgerTxEvent;
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
use spectrum_cardano_lib::time::slot_to_time_millis;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::NetworkId;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::event_sink::event_handler::EventHandler;
use std::collections::BTreeSet;

#[derive(Clone)]
pub struct TxHandler<Index> {
    index: Index,
    network_id: NetworkId,
    tracked_limit_order_script_hashes: BTreeSet<String>,
}

impl<Index> TxHandler<Index> {
    pub fn new(index: Index, network_id: NetworkId, tracked_limit_order_script_hashes: Vec<String>) -> Self {
        Self {
            index,
            network_id,
            tracked_limit_order_script_hashes: tracked_limit_order_script_hashes.into_iter().collect(),
        }
    }
}

#[async_trait]
impl<Index> EventHandler<LedgerTxEvent<TxViewMut>> for TxHandler<Index>
where
    Index: ExecutionIndex + Clone + Send + Sync + 'static,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent<TxViewMut>) -> Option<LedgerTxEvent<TxViewMut>> {
        match ev {
            LedgerTxEvent::TxApplied {
                tx,
                slot,
                block_number,
                ..
            } => {
                apply_tx(
                    &self.index,
                    self.network_id,
                    &self.tracked_limit_order_script_hashes,
                    tx,
                    slot,
                    block_number,
                )
                .await
            }
            LedgerTxEvent::TxUnapplied { tx, .. } => {
                self.index.unapply_tx(&tx.hash.to_hex()).await;
            }
        }
        None
    }
}

async fn apply_tx<Index>(
    index: &Index,
    network_id: NetworkId,
    tracked_limit_order_script_hashes: &BTreeSet<String>,
    tx: TxViewMut,
    slot: u64,
    block_number: u64,
) where
    Index: ExecutionIndex + Send + Sync,
{
    let time_ms = Some(slot_to_time_millis(slot, network_id));
    let snapshot = index.snapshot().await;
    let consumed = tx
        .inputs
        .iter()
        .cloned()
        .map(OutputRef::from)
        .map(order_ref_from_output_ref)
        .collect::<Vec<_>>();

    let consumed_orders = snapshot
        .orders
        .iter()
        .filter(|order| consumed.iter().any(|candidate| candidate == &order.order_ref))
        .cloned()
        .collect::<Vec<_>>();

    if !consumed_orders.is_empty() {
        let attribution = classify_attribution(&tx.signers, &consumed_orders);
        let classification = match attribution {
            ExecutionAttribution::Single(_)
            | ExecutionAttribution::Ambiguous(_)
            | ExecutionAttribution::UnknownPermissionless => ExecutionClassification::Executed,
            ExecutionAttribution::Unknown => ExecutionClassification::UnknownSpent,
        };
        let status = match classification {
            ExecutionClassification::Executed => OrderStatus::Executed {
                execution_tx: tx.hash.to_hex(),
                attribution: attribution.clone(),
                slot,
                time_ms,
            },
            ExecutionClassification::UnknownSpent => OrderStatus::UnknownSpent {
                tx: tx.hash.to_hex(),
                slot,
                time_ms,
                reason: "confirmed spend did not contain exactly one permitted executor signer".to_string(),
            },
            ExecutionClassification::Cancelled => {
                unreachable!("cancellation is not classified in ledger-only v1")
            }
        };
        for order in &consumed_orders {
            index.mark_order_spent(&order.order_ref, status.clone()).await;
        }
        index
            .upsert_execution(ExecutionRecord {
                tx_hash: tx.hash.to_hex(),
                slot,
                time_ms,
                attribution: attribution.clone(),
                signer_batchers: signer_batchers_for_record(&attribution, &tx.signers),
                consumed_orders: consumed_orders
                    .iter()
                    .map(|order| order.order_ref.clone())
                    .collect(),
                pair: consumed_orders.first().map(|order| order.pair.clone()),
                classification,
            })
            .await;
        register_attributed_batchers(index, attribution, &tx.signers, slot, time_ms).await;
    }

    for (output_index, output) in tx.outputs {
        if !is_tracked_limit_order_output(&output, tracked_limit_order_script_hashes) {
            continue;
        }
        if let Some(order) = LimitOrderObservation::try_from_output(&output) {
            let input_asset = order.input_asset.to_string();
            let output_asset = order.output_asset.to_string();
            let observed = ObservedOrder {
                order_ref: OrderRef {
                    tx_hash: tx.hash.to_hex(),
                    output_index: output_index as u64,
                },
                kind: OrderKind::Limit,
                pair: pair_id(&input_asset, &output_asset),
                input_asset,
                output_asset,
                input_amount: order.tradable_input,
                output_amount: Some(order.output_amount),
                price_num: (*order.base_price.numer()).min(u64::MAX as u128) as u64,
                price_denom: (*order.base_price.denom()).min(u64::MAX as u128) as u64,
                permitted_executors: order.permitted_executors.iter().map(key_hash_to_hex).collect(),
                created_slot: slot,
                created_time_ms: time_ms,
                status: OrderStatus::Open,
            };
            for executor in &observed.permitted_executors {
                index
                    .upsert_batcher(BatcherProfile {
                        pkh: executor.clone(),
                        first_seen_slot: slot,
                        first_seen_ms: time_ms,
                        last_seen_slot: slot,
                        last_seen_ms: time_ms,
                        source: BatcherDiscoverySource::PermittedExecutor,
                    })
                    .await;
            }
            index.upsert_order(observed).await;
        }
    }

    index.set_tip(slot, block_number).await;
}

fn classify_attribution(
    signers: &[Ed25519KeyHash],
    consumed_orders: &[ObservedOrder],
) -> ExecutionAttribution {
    let mut permitted = consumed_orders
        .iter()
        .flat_map(|order| order.permitted_executors.iter().cloned())
        .collect::<Vec<_>>();
    permitted.sort();
    permitted.dedup();

    if permitted.is_empty() {
        return ExecutionAttribution::UnknownPermissionless;
    }

    let mut matching = signers
        .iter()
        .map(key_hash_to_hex)
        .filter(|signer| permitted.iter().any(|executor| executor == signer))
        .collect::<Vec<_>>();
    matching.sort();
    matching.dedup();

    match matching.len() {
        0 => ExecutionAttribution::Unknown,
        1 => ExecutionAttribution::Single(matching.remove(0)),
        _ => ExecutionAttribution::Ambiguous(matching),
    }
}

async fn register_attributed_batchers<Index>(
    index: &Index,
    attribution: ExecutionAttribution,
    tx_signers: &[Ed25519KeyHash],
    slot: u64,
    time_ms: Option<u64>,
) where
    Index: ExecutionIndex + Send + Sync,
{
    let batchers = match attribution {
        ExecutionAttribution::Single(pkh) => vec![pkh],
        ExecutionAttribution::Ambiguous(pkhs) => pkhs,
        ExecutionAttribution::Unknown => Vec::new(),
        ExecutionAttribution::UnknownPermissionless => tx_signers.iter().map(key_hash_to_hex).collect(),
    };

    for pkh in batchers {
        index
            .upsert_batcher(BatcherProfile {
                pkh,
                first_seen_slot: slot,
                first_seen_ms: time_ms,
                last_seen_slot: slot,
                last_seen_ms: time_ms,
                source: BatcherDiscoverySource::ExecutionSigner,
            })
            .await;
    }
}

fn order_ref_from_output_ref(output_ref: OutputRef) -> OrderRef {
    OrderRef {
        tx_hash: output_ref.tx_hash().to_hex(),
        output_index: output_ref.index(),
    }
}

fn key_hash_to_hex(key_hash: &Ed25519KeyHash) -> String {
    key_hash.to_raw_hex()
}

fn signer_batchers_for_record(
    attribution: &ExecutionAttribution,
    tx_signers: &[Ed25519KeyHash],
) -> Vec<String> {
    match attribution {
        ExecutionAttribution::Single(pkh) => vec![pkh.clone()],
        ExecutionAttribution::Ambiguous(pkhs) => pkhs.clone(),
        ExecutionAttribution::Unknown => vec![],
        ExecutionAttribution::UnknownPermissionless => tx_signers.iter().map(key_hash_to_hex).collect(),
    }
}

fn is_tracked_limit_order_output(
    output: &cml_chain::transaction::TransactionOutput,
    tracked_limit_order_script_hashes: &BTreeSet<String>,
) -> bool {
    output
        .script_hash()
        .map(|hash| tracked_limit_order_script_hashes.contains(&hash.to_raw_hex()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(permitted_executors: Vec<String>) -> ObservedOrder {
        ObservedOrder {
            order_ref: OrderRef {
                tx_hash: "tx".to_string(),
                output_index: 0,
            },
            kind: OrderKind::Limit,
            pair: "a/b".to_string(),
            input_asset: "a".to_string(),
            output_asset: "b".to_string(),
            input_amount: 1,
            output_amount: Some(1),
            price_num: 1,
            price_denom: 1,
            permitted_executors,
            created_slot: 0,
            created_time_ms: Some(0),
            status: OrderStatus::Open,
        }
    }

    #[test]
    fn attribution_uses_intersection_of_signers_and_permitted_executors() {
        let signer = Ed25519KeyHash::from([1; 28]);
        let other = Ed25519KeyHash::from([2; 28]);
        let orders = vec![observed(vec![key_hash_to_hex(&signer)])];

        assert_eq!(
            classify_attribution(&[signer, other], &orders),
            ExecutionAttribution::Single(key_hash_to_hex(&signer))
        );
    }
}
