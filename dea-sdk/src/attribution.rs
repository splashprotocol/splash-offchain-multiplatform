use crate::domain::{
    ActorId, AttributionConfidence, AttributionRecord, SteeredOrderContext, TransactionObservation,
};

pub fn attribute_actor(
    tx: &TransactionObservation,
    ctx: Option<&SteeredOrderContext>,
) -> AttributionRecord {
    let signer_count = tx.signers.len();
    if let Some(ctx) = ctx {
        let matches: Vec<ActorId> = tx
            .signers
            .iter()
            .filter_map(|s| {
                if ctx.permitted_executors.iter().any(|e| e == &s.pkh) {
                    Some(s.pkh.clone())
                } else {
                    None
                }
            })
            .collect();
        if matches.len() == 1 {
            return AttributionRecord {
                actor_id: Some(matches[0].clone()),
                confidence: if signer_count == 1 {
                    AttributionConfidence::High
                } else {
                    AttributionConfidence::Medium
                },
                matched_permitted_executor: Some(matches[0].clone()),
                signer_count,
            };
        }
    }

    if signer_count == 1 {
        return AttributionRecord {
            actor_id: Some(tx.signers[0].pkh.clone()),
            confidence: AttributionConfidence::High,
            matched_permitted_executor: None,
            signer_count,
        };
    }

    AttributionRecord {
        actor_id: Some("unknown_multi_sig".to_string()),
        confidence: AttributionConfidence::Unknown,
        matched_permitted_executor: None,
        signer_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        InteractionRole, MarketTouch, SignerCredential, TradeDirection, TransactionObservation,
    };

    fn base_tx(signers: &[&str]) -> TransactionObservation {
        TransactionObservation {
            tx_hash: "tx".into(),
            mempool: None,
            ledger: None,
            signers: signers
                .iter()
                .map(|s| SignerCredential { pkh: (*s).into() })
                .collect(),
            touches: vec![MarketTouch {
                pool_id: "pool".into(),
                pair_id: "a/b".into(),
                role: InteractionRole::OrderExecution,
                direction: TradeDirection::Buy,
                base_asset: "a".into(),
                quote_asset: "b".into(),
                input_amount: 1,
                output_amount: Some(1),
                output_loss_asset: None,
                output_loss_amount: None,
                counterfactual_output_amount: None,
                executable_after: Some(true),
                state_displacement_bps: 0,
                net_base_flow: -1,
                net_quote_flow: 1,
                depends_on_txs: vec![],
            }],
            steered_order: None,
            attribution: None,
        }
    }

    #[test]
    fn steered_order_match_is_preferred() {
        let tx = base_tx(&["batcher-1", "aux"]);
        let ctx = SteeredOrderContext {
            order_ref: None,
            permitted_executors: vec!["batcher-1".into()],
        };
        let attr = attribute_actor(&tx, Some(&ctx));
        assert_eq!(attr.actor_id.as_deref(), Some("batcher-1"));
        assert_eq!(attr.confidence, AttributionConfidence::Medium);
    }

    #[test]
    fn single_signer_maps_high_confidence() {
        let tx = base_tx(&["solo"]);
        let attr = attribute_actor(&tx, None);
        assert_eq!(attr.actor_id.as_deref(), Some("solo"));
        assert_eq!(attr.confidence, AttributionConfidence::High);
    }

    #[test]
    fn multi_sig_is_unknown() {
        let tx = base_tx(&["a", "b"]);
        let attr = attribute_actor(&tx, None);
        assert_eq!(attr.actor_id.as_deref(), Some("unknown_multi_sig"));
        assert_eq!(attr.confidence, AttributionConfidence::Unknown);
    }
}

