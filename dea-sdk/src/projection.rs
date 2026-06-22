use crate::domain::{pair_id, MarketTouch, PoolId, TransactionObservation};

pub fn exact_pool_ids(tx: &TransactionObservation) -> Vec<PoolId> {
    tx.touches.iter().map(|t| t.pool_id.clone()).collect()
}

pub fn unique_exact_pool_id(tx: &TransactionObservation) -> Option<PoolId> {
    let mut ids = exact_pool_ids(tx);
    ids.sort();
    ids.dedup();
    if ids.len() == 1 {
        ids.into_iter().next()
    } else {
        None
    }
}

pub fn touches_same_exact_pool(a: &TransactionObservation, b: &TransactionObservation) -> bool {
    matches!(
        (unique_exact_pool_id(a), unique_exact_pool_id(b)),
        (Some(a_pool), Some(b_pool)) if a_pool == b_pool
    )
}

pub fn canonical_pair_from_touch(touch: &MarketTouch) -> String {
    pair_id(&touch.base_asset, &touch.quote_asset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        InteractionRole, LedgerObservation, MarketTouch, MempoolObservation, TradeDirection,
        TransactionObservation,
    };

    fn tx(hash: &str, pools: &[&str]) -> TransactionObservation {
        TransactionObservation {
            tx_hash: hash.to_string(),
            mempool: Some(MempoolObservation {
                first_seen_at: 1,
                last_seen_at: None,
                mempool_sequence: 1,
            }),
            ledger: Some(LedgerObservation {
                confirmed_slot: Some(1),
                confirmed_block_hash: Some("block".into()),
                confirmed_tx_index: Some(0),
            }),
            signers: vec![],
            touches: pools
                .iter()
                .map(|pool| MarketTouch {
                    pool_id: (*pool).to_string(),
                    pair_id: "aaa/bbb".into(),
                    role: InteractionRole::DirectSwap,
                    direction: TradeDirection::Buy,
                    base_asset: "aaa".into(),
                    quote_asset: "bbb".into(),
                    input_amount: 1,
                    output_amount: Some(1),
                    output_loss_asset: None,
                    output_loss_amount: None,
                    counterfactual_output_amount: None,
                    executable_after: Some(true),
                    state_displacement_bps: 10,
                    net_base_flow: -1,
                    net_quote_flow: 1,
                    depends_on_txs: vec![],
                })
                .collect(),
            steered_order: None,
            attribution: None,
        }
    }

    #[test]
    fn same_pool_match_is_exact() {
        assert!(touches_same_exact_pool(&tx("a", &["pool-1"]), &tx("b", &["pool-1"])));
        assert!(!touches_same_exact_pool(&tx("a", &["pool-1"]), &tx("b", &["pool-2"])));
    }

    #[test]
    fn ambiguous_multi_pool_is_rejected() {
        assert_eq!(unique_exact_pool_id(&tx("a", &["pool-1", "pool-2"])), None);
    }
}

