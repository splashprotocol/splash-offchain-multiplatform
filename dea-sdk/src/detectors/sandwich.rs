use crate::config::DeaConfig;
use crate::domain::{
    fixed_4, AttackerBenefitMetrics, AttributionConfidence, EvidenceKind, EvidenceRef,
    SandwichCandidate, SandwichFinding, TransactionObservation, VictimHarmMetrics,
};
use crate::projection::touches_same_exact_pool;

fn confidence_rank(value: AttributionConfidence) -> u8 {
    match value {
        AttributionConfidence::Unknown => 0,
        AttributionConfidence::Low => 1,
        AttributionConfidence::Medium => 2,
        AttributionConfidence::High => 3,
    }
}

fn tx_order_key(tx: &TransactionObservation) -> Option<(u64, u32)> {
    tx.ledger.as_ref().and_then(|l| {
        l.confirmed_slot
            .zip(l.confirmed_tx_index)
            .map(|(slot, index)| (slot, index))
    })
}

fn is_directional_role(role: &crate::domain::InteractionRole) -> bool {
    matches!(
        role,
        crate::domain::InteractionRole::OrderCreation
            | crate::domain::InteractionRole::OrderExecution
            | crate::domain::InteractionRole::DirectSwap
    )
}

pub fn detect_sandwich_candidates(
    cfg: &DeaConfig,
    observations: &[TransactionObservation],
) -> Vec<SandwichCandidate> {
    let mut out = Vec::new();
    for victim in observations {
        let Some(victim_pool) = victim.touches.first() else {
            continue;
        };
        for pre in observations {
            if pre.tx_hash == victim.tx_hash {
                continue;
            }
            if !touches_same_exact_pool(pre, victim) {
                continue;
            }
            let Some(pre_m) = pre.mempool.as_ref() else {
                continue;
            };
            let Some(victim_m) = victim.mempool.as_ref() else {
                continue;
            };
            if pre_m.first_seen_at >= victim_m.first_seen_at {
                continue;
            }
            for post in observations {
                if post.tx_hash == victim.tx_hash || post.tx_hash == pre.tx_hash {
                    continue;
                }
                if !touches_same_exact_pool(post, victim) {
                    continue;
                }
                let Some(post_m) = post.mempool.as_ref() else {
                    continue;
                };
                if post_m.first_seen_at <= victim_m.first_seen_at {
                    continue;
                }
                let pre_actor = pre.attribution.as_ref().and_then(|a| a.actor_id.clone());
                let post_actor = post.attribution.as_ref().and_then(|a| a.actor_id.clone());
                let victim_actor = victim.attribution.as_ref().and_then(|a| a.actor_id.clone());
                if pre_actor.is_none() || pre_actor != post_actor || pre_actor == victim_actor {
                    continue;
                }
                let pre_conf = pre
                    .attribution
                    .as_ref()
                    .map(|a| a.confidence)
                    .unwrap_or(AttributionConfidence::Unknown);
                let post_conf = post
                    .attribution
                    .as_ref()
                    .map(|a| a.confidence)
                    .unwrap_or(AttributionConfidence::Unknown);
                if confidence_rank(pre_conf) < confidence_rank(cfg.min_actor_attribution_confidence)
                    || confidence_rank(post_conf)
                        < confidence_rank(cfg.min_actor_attribution_confidence)
                {
                    continue;
                }
                let pret = &pre.touches[0];
                let victimt = &victim.touches[0];
                let postt = &post.touches[0];
                if !is_directional_role(&pret.role)
                    || !is_directional_role(&victimt.role)
                    || !is_directional_role(&postt.role)
                {
                    continue;
                }
                if pret.direction == postt.direction {
                    continue;
                }
                out.push(SandwichCandidate {
                    pre_tx: pre.tx_hash.clone(),
                    victim_tx: victim.tx_hash.clone(),
                    post_tx: post.tx_hash.clone(),
                    victim_first_seen_at: victim_m.first_seen_at,
                    attacker_actor: pre_actor,
                    pool_id: victim_pool.pool_id.clone(),
                    pair_id: victim_pool.pair_id.clone(),
                    evidence_refs: vec![EvidenceRef {
                        tx_hash: victim.tx_hash.clone(),
                        kind: EvidenceKind::MempoolFirstSeen,
                        note: "victim is bracketed by same-actor opposite-direction trades".into(),
                    }],
                });
            }
        }
    }
    out
}

pub fn confirm_sandwiches(
    cfg: &DeaConfig,
    observations: &[TransactionObservation],
    candidates: &[SandwichCandidate],
) -> Vec<SandwichFinding> {
    let mut out = Vec::new();
    for candidate in candidates {
        let pre = observations.iter().find(|tx| tx.tx_hash == candidate.pre_tx).unwrap();
        let victim = observations.iter().find(|tx| tx.tx_hash == candidate.victim_tx).unwrap();
        let post = observations.iter().find(|tx| tx.tx_hash == candidate.post_tx).unwrap();
        let pret = &pre.touches[0];
        let vt = &victim.touches[0];
        let postt = &post.touches[0];
        if !is_directional_role(&pret.role)
            || !is_directional_role(&vt.role)
            || !is_directional_role(&postt.role)
        {
            continue;
        }
        let Some(pre_key) = tx_order_key(pre) else {
            continue;
        };
        let Some(victim_key) = tx_order_key(victim) else {
            continue;
        };
        let Some(post_key) = tx_order_key(post) else {
            continue;
        };
        if !(pre_key < victim_key && victim_key < post_key) {
            continue;
        }
        if victim.attribution.as_ref().and_then(|a| a.actor_id.clone()) == candidate.attacker_actor {
            continue;
        }

        let output_loss_amount = vt.output_loss_amount;
        let price_degradation_bps = match (vt.counterfactual_output_amount, vt.output_amount) {
            (Some(counter), Some(actual)) if counter > actual => {
                Some((((counter - actual) as u128) * 10_000 / counter as u128) as u64)
            }
            _ => None,
        };
        let unwound_base = pret
            .net_base_flow
            .unsigned_abs()
            .min(postt.net_base_flow.unsigned_abs()) as u128;
        let pre_base = pret.net_base_flow.unsigned_abs() as u128;
        let unwind_ratio_bps =
            if pre_base == 0 { None } else { Some((unwound_base * 10_000 / pre_base) as u64) };
        if unwind_ratio_bps.unwrap_or(0) < cfg.min_unwind_ratio_bps {
            continue;
        }

        let estimated_profit = if pret.quote_asset == postt.quote_asset {
            Some(pret.net_quote_flow + postt.net_quote_flow)
        } else {
            None
        };
        if price_degradation_bps.unwrap_or(0) < cfg.min_victim_harm_bps {
            continue;
        }
        if matches!(estimated_profit, Some(v) if v <= 0) {
            continue;
        }
        let mut confidence = 0.25 + 0.20 + 0.20 + 0.15;
        confidence += 0.10;
        if matches!(estimated_profit, Some(v) if v > 0) {
            confidence += 0.10;
        }
        if confidence < 0.50 {
            continue;
        }
        out.push(SandwichFinding {
            pre_tx: pre.tx_hash.clone(),
            victim_tx: victim.tx_hash.clone(),
            post_tx: post.tx_hash.clone(),
            attacker_actor: candidate.attacker_actor.clone().unwrap(),
            pair_ids: vec![candidate.pair_id.clone()],
            pool_ids: vec![candidate.pool_id.clone()],
            victim_harm_metrics: VictimHarmMetrics {
                price_degradation_bps,
                output_loss_asset: vt.output_loss_asset.clone(),
                output_loss_amount,
                baseline_computable: vt.counterfactual_output_amount.is_some(),
            },
            attacker_benefit_metrics: AttackerBenefitMetrics {
                quote_asset: Some(pret.quote_asset.clone()),
                estimated_gross_profit: estimated_profit,
                unwind_ratio_bps,
            },
            confidence: fixed_4(confidence),
            evidence_refs: vec![
                EvidenceRef {
                    tx_hash: pre.tx_hash.clone(),
                    kind: EvidenceKind::LedgerConfirmation,
                    note: "pre-leg before victim".into(),
                },
                EvidenceRef {
                    tx_hash: post.tx_hash.clone(),
                    kind: EvidenceKind::LedgerConfirmation,
                    note: "post-leg after victim".into(),
                },
            ],
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribution::attribute_actor;
    use crate::aggregation::aggregate_actor_fairness;
    use crate::domain::{AssessmentWindow, RiskBand};
    use crate::domain::{
        InteractionRole, LedgerObservation, MarketTouch, MempoolObservation, SignerCredential,
        TradeDirection, TransactionObservation,
    };

    fn tx(
        hash: &str,
        actor: &str,
        first_seen: u64,
        slot: u64,
        pool: &str,
        direction: TradeDirection,
        net_base_flow: i128,
        net_quote_flow: i128,
        counterfactual_output_amount: Option<u128>,
        output_amount: Option<u128>,
        output_loss_amount: Option<u128>,
    ) -> TransactionObservation {
        let mut tx = TransactionObservation {
            tx_hash: hash.into(),
            mempool: Some(MempoolObservation {
                first_seen_at: first_seen,
                last_seen_at: None,
                mempool_sequence: first_seen,
            }),
            ledger: Some(LedgerObservation {
                confirmed_slot: Some(slot),
                confirmed_block_hash: Some("block".into()),
                confirmed_tx_index: Some(slot as u32),
            }),
            signers: vec![SignerCredential { pkh: actor.into() }],
            touches: vec![MarketTouch {
                pool_id: pool.into(),
                pair_id: "a/b".into(),
                role: InteractionRole::DirectSwap,
                direction,
                base_asset: "a".into(),
                quote_asset: "b".into(),
                input_amount: 100,
                output_amount,
                output_loss_asset: Some("b".into()),
                output_loss_amount,
                counterfactual_output_amount,
                executable_after: Some(true),
                state_displacement_bps: 120,
                net_base_flow,
                net_quote_flow,
                depends_on_txs: vec![],
            }],
            steered_order: None,
            attribution: None,
        };
        tx.attribution = Some(attribute_actor(&tx, None));
        tx
    }

    #[test]
    fn detects_same_pool_same_actor_bracket() {
        let cfg = DeaConfig::default();
        let pre = tx("pre", "attacker", 10, 1, "pool-1", TradeDirection::Buy, -100, -100, None, Some(50), None);
        let victim = tx(
            "victim",
            "victim",
            11,
            2,
            "pool-1",
            TradeDirection::Buy,
            -100,
            90,
            Some(100),
            Some(90),
            Some(10),
        );
        let post = tx("post", "attacker", 12, 3, "pool-1", TradeDirection::Sell, 100, 130, None, Some(60), None);
        let candidates = detect_sandwich_candidates(&cfg, &[pre.clone(), victim.clone(), post.clone()]);
        assert_eq!(candidates.len(), 1);
        let findings = confirm_sandwiches(&cfg, &[pre, victim, post], &candidates);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].attacker_actor, "attacker");
    }

    #[test]
    fn rejects_different_pool_triple() {
        let cfg = DeaConfig::default();
        let pre = tx("pre", "attacker", 10, 1, "pool-1", TradeDirection::Buy, -100, -100, None, Some(50), None);
        let victim = tx("victim", "victim", 11, 2, "pool-2", TradeDirection::Buy, -100, 90, Some(100), Some(90), Some(10));
        let post = tx("post", "attacker", 12, 3, "pool-1", TradeDirection::Sell, 100, 130, None, Some(60), None);
        let candidates = detect_sandwich_candidates(&cfg, &[pre, victim, post]);
        assert!(candidates.is_empty());
    }

    #[test]
    fn candidate_alone_is_not_actor_escalation() {
        let cfg = DeaConfig::default();
        let pre = tx("pre", "attacker", 10, 1, "pool-1", TradeDirection::Buy, -100, -100, None, Some(50), None);
        let victim = tx("victim", "victim", 11, 2, "pool-1", TradeDirection::Buy, -100, 90, Some(100), Some(90), Some(10));
        let post = tx("post", "attacker", 12, 3, "pool-1", TradeDirection::Sell, 100, 130, None, Some(60), None);
        let candidates = detect_sandwich_candidates(&cfg, &[pre.clone(), victim.clone(), post.clone()]);
        assert_eq!(candidates.len(), 1);
        let findings = confirm_sandwiches(&cfg, &[pre, victim, post], &candidates);
        assert_eq!(findings.len(), 1);
        let profiles = aggregate_actor_fairness(
            &cfg,
            &AssessmentWindow { from_ms: 0, to_ms: 200_000_000 },
            &[],
            &[],
            &candidates,
            &findings,
        );
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].risk_band, RiskBand::Low);
    }

    #[test]
    fn same_block_tx_index_allows_bracketing() {
        let cfg = DeaConfig::default();
        let mut pre = tx("pre", "attacker", 10, 5, "pool-1", TradeDirection::Buy, -100, -100, None, Some(50), None);
        let mut victim = tx("victim", "victim", 11, 5, "pool-1", TradeDirection::Buy, -100, 90, Some(100), Some(90), Some(10));
        let mut post = tx("post", "attacker", 12, 5, "pool-1", TradeDirection::Sell, 100, 130, None, Some(60), None);
        pre.ledger.as_mut().unwrap().confirmed_tx_index = Some(0);
        victim.ledger.as_mut().unwrap().confirmed_tx_index = Some(1);
        post.ledger.as_mut().unwrap().confirmed_tx_index = Some(2);
        let candidates = detect_sandwich_candidates(&cfg, &[pre.clone(), victim.clone(), post.clone()]);
        let findings = confirm_sandwiches(&cfg, &[pre, victim, post], &candidates);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn sandwich_without_normalizable_profit_uses_structural_confidence_only() {
        let cfg = DeaConfig::default();
        let mut pre = tx("pre", "attacker", 10, 1, "pool-1", TradeDirection::Buy, -100, -100, None, Some(50), None);
        let victim = tx("victim", "victim", 11, 2, "pool-1", TradeDirection::Buy, -100, 90, Some(100), Some(90), Some(10));
        let mut post = tx("post", "attacker", 12, 3, "pool-1", TradeDirection::Sell, 100, 130, None, Some(60), None);
        pre.touches[0].quote_asset = "b".into();
        post.touches[0].quote_asset = "c".into();
        let candidates = detect_sandwich_candidates(&cfg, &[pre.clone(), victim.clone(), post.clone()]);
        let findings = confirm_sandwiches(&cfg, &[pre, victim, post], &candidates);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].attacker_benefit_metrics.estimated_gross_profit, None);
        assert_eq!(findings[0].confidence, "0.9000");
    }
}
