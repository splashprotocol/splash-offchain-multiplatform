use crate::config::DeaConfig;
use crate::domain::{
    fixed_4, EvidenceKind, EvidenceRef, FrontRunCandidate, FrontRunFinding, FrontRunHarmKind,
    FrontRunHarmMetrics, TransactionObservation,
};
use crate::projection::touches_same_exact_pool;

fn confidence_value(s: &str) -> f64 {
    s.parse::<f64>().unwrap_or(0.0)
}

fn min_confidence_floor(finding: &FrontRunFinding) -> f64 {
    match finding.victim_harm {
        FrontRunHarmKind::StatePriorityOnly => 0.55,
        FrontRunHarmKind::WorsePrice | FrontRunHarmKind::BecameNonExecutable => 0.65,
    }
}

fn is_potential_victim(tx: &TransactionObservation) -> bool {
    tx.touches.iter().any(|touch| {
        touch.counterfactual_output_amount.is_some()
            || touch.output_loss_amount.is_some()
            || touch.executable_after == Some(false)
    })
}

fn tx_order_key(tx: &TransactionObservation) -> Option<(u64, u32)> {
    tx.ledger.as_ref().and_then(|l| {
        l.confirmed_slot
            .zip(l.confirmed_tx_index)
            .map(|(slot, index)| (slot, index))
    })
}

pub fn detect_front_run_candidates(
    cfg: &DeaConfig,
    observations: &[TransactionObservation],
) -> Vec<FrontRunCandidate> {
    let mut out = Vec::new();
    for victim in observations {
        let Some(victim_mempool) = victim.mempool.as_ref() else {
            continue;
        };
        if victim.touches.is_empty() || !is_potential_victim(victim) {
            continue;
        }
        for suspect in observations {
            if victim.tx_hash == suspect.tx_hash {
                continue;
            }
            let Some(suspect_mempool) = suspect.mempool.as_ref() else {
                continue;
            };
            if suspect_mempool.first_seen_at <= victim_mempool.first_seen_at {
                continue;
            }
            let delta = suspect_mempool
                .first_seen_at
                .saturating_sub(victim_mempool.first_seen_at);
            if delta > cfg.front_run_window_ms {
                continue;
            }
            if delta <= cfg.max_same_tick_ambiguity {
                continue;
            }
            if !touches_same_exact_pool(victim, suspect) {
                continue;
            }
            let vtouch = &victim.touches[0];
            let stouch = &suspect.touches[0];
            if vtouch.input_amount < cfg.min_suspect_input_amount
                || stouch.input_amount < cfg.min_suspect_input_amount
            {
                continue;
            }
            if matches!(
                stouch.role,
                crate::domain::InteractionRole::LiquidityChange
                    | crate::domain::InteractionRole::Admin
                    | crate::domain::InteractionRole::Cancel
            ) {
                continue;
            }
            if vtouch.direction != stouch.direction
                || stouch.state_displacement_bps >= cfg.min_state_displacement_bps
            {
                out.push(FrontRunCandidate {
                    victim_tx: victim.tx_hash.clone(),
                    suspect_tx: suspect.tx_hash.clone(),
                    victim_first_seen_at: victim_mempool.first_seen_at,
                    pool_id: vtouch.pool_id.clone(),
                    pair_id: vtouch.pair_id.clone(),
                    first_seen_delta_ms: delta,
                    victim_actor: victim.attribution.as_ref().and_then(|a| a.actor_id.clone()),
                    suspect_actor: suspect.attribution.as_ref().and_then(|a| a.actor_id.clone()),
                    evidence_refs: vec![
                        EvidenceRef {
                            tx_hash: victim.tx_hash.clone(),
                            kind: EvidenceKind::MempoolFirstSeen,
                            note: "victim seen first".into(),
                        },
                        EvidenceRef {
                            tx_hash: suspect.tx_hash.clone(),
                            kind: EvidenceKind::MempoolFirstSeen,
                            note: "suspect seen later on same pool".into(),
                        },
                    ],
                });
            }
        }
    }
    out
}

pub fn confirm_front_runs(
    cfg: &DeaConfig,
    observations: &[TransactionObservation],
    candidates: &[FrontRunCandidate],
) -> Vec<FrontRunFinding> {
    let mut out = Vec::new();
    for candidate in candidates {
        let Some(victim) = observations.iter().find(|tx| tx.tx_hash == candidate.victim_tx) else {
            continue;
        };
        let Some(suspect) = observations.iter().find(|tx| tx.tx_hash == candidate.suspect_tx) else {
            continue;
        };
        let vtouch = &victim.touches[0];
        let stouch = &suspect.touches[0];
        if candidate.victim_actor.is_some() && candidate.victim_actor == candidate.suspect_actor {
            continue;
        }
        let suspect_confirmed = tx_order_key(suspect).is_some();
        let suspect_before_victim = matches!(
            (tx_order_key(suspect), tx_order_key(victim)),
            (Some(s_order), Some(v_order)) if s_order < v_order
        );
        let mut confidence = 0.0;
        let mut evidence = candidate.evidence_refs.clone();
        confidence += 0.35;
        if suspect_before_victim {
            confidence += 0.20;
            evidence.push(EvidenceRef {
                tx_hash: suspect.tx_hash.clone(),
                kind: EvidenceKind::LedgerConfirmation,
                note: "suspect confirmed before victim".into(),
            });
        }
        confidence += 0.20;
        let counterfactual = vtouch.counterfactual_output_amount;
        let actual = vtouch.output_amount;
        let mut harm = FrontRunHarmKind::StatePriorityOnly;
        let mut price_degradation_bps = None;
        let mut counterfactual_computable = false;
        if suspect_before_victim {
            if let (Some(counter), Some(act)) = (counterfactual, actual) {
            counterfactual_computable = true;
            if counter > act {
                let bps = (((counter - act) as u128) * 10_000 / counter as u128) as u64;
                if bps >= cfg.min_price_impact_bps {
                    harm = FrontRunHarmKind::WorsePrice;
                    price_degradation_bps = Some(bps);
                    confidence += 0.15;
                    evidence.push(EvidenceRef {
                        tx_hash: victim.tx_hash.clone(),
                        kind: EvidenceKind::CounterfactualSimulation,
                        note: format!("victim lost {bps} bps versus counterfactual"),
                    });
                }
            }
            }
        }
        if matches!(harm, FrontRunHarmKind::StatePriorityOnly)
            && suspect_confirmed
            && victim.ledger.as_ref().and_then(|l| l.confirmed_slot).is_none()
            && vtouch.executable_after == Some(false)
            && stouch.state_displacement_bps >= cfg.min_state_displacement_bps
        {
            harm = FrontRunHarmKind::BecameNonExecutable;
            confidence += 0.15;
            evidence.push(EvidenceRef {
                tx_hash: suspect.tx_hash.clone(),
                kind: EvidenceKind::PoolStateDelta,
                note: "suspect state displacement made victim non-executable".into(),
            });
        } else if suspect_before_victim && stouch.state_displacement_bps >= cfg.min_state_displacement_bps {
            evidence.push(EvidenceRef {
                tx_hash: suspect.tx_hash.clone(),
                kind: EvidenceKind::PoolStateDelta,
                note: "ordering suspicion with measurable state displacement".into(),
            });
        } else {
            continue;
        }

        let execution_slot_delta = suspect
            .ledger
            .as_ref()
            .and_then(|s| s.confirmed_slot)
            .zip(victim.ledger.as_ref().and_then(|v| v.confirmed_slot))
            .map(|(s, v)| s as i64 - v as i64);

        let finding = FrontRunFinding {
            victim_tx: victim.tx_hash.clone(),
            suspect_tx: suspect.tx_hash.clone(),
            victim_first_seen_at: victim.mempool.as_ref().unwrap().first_seen_at,
            suspect_first_seen_at: suspect.mempool.as_ref().unwrap().first_seen_at,
            first_seen_delta_ms: candidate.first_seen_delta_ms,
            victim_confirmed_slot: victim.ledger.as_ref().and_then(|l| l.confirmed_slot),
            suspect_confirmed_slot: suspect.ledger.as_ref().and_then(|l| l.confirmed_slot),
            pair_ids: vec![candidate.pair_id.clone()],
            pool_ids: vec![candidate.pool_id.clone()],
            victim_harm: harm,
            harm_metrics: FrontRunHarmMetrics {
                price_degradation_bps,
                execution_slot_delta,
                counterfactual_computable,
            },
            suspect_actor: candidate.suspect_actor.clone(),
            confidence: fixed_4(confidence),
            evidence_refs: evidence,
        };
        if confidence_value(&finding.confidence) >= min_confidence_floor(&finding) {
            out.push(finding);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribution::attribute_actor;
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
        actual: Option<u128>,
        counterfactual: Option<u128>,
        executable_after: Option<bool>,
        state_bps: u64,
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
                output_amount: actual,
                output_loss_asset: Some("b".into()),
                output_loss_amount: counterfactual.zip(actual).map(|(c, a)| c.saturating_sub(a)),
                counterfactual_output_amount: counterfactual,
                executable_after,
                state_displacement_bps: state_bps,
                net_base_flow: -100,
                net_quote_flow: actual.unwrap_or(0) as i128,
                depends_on_txs: vec![],
            }],
            steered_order: None,
            attribution: None,
        };
        tx.attribution = Some(attribute_actor(&tx, None));
        tx
    }

    #[test]
    fn candidate_generation_selects_later_seen_same_pool_only() {
        let cfg = DeaConfig::default();
        let victim = tx(
            "victim",
            "victim-actor",
            10,
            5,
            "pool-1",
            TradeDirection::Buy,
            Some(90),
            Some(100),
            Some(true),
            100,
        );
        let suspect = tx(
            "suspect",
            "attacker",
            11,
            4,
            "pool-1",
            TradeDirection::Sell,
            Some(95),
            None,
            Some(true),
            150,
        );
        let other_pool = tx(
            "other",
            "attacker-2",
            12,
            3,
            "pool-2",
            TradeDirection::Sell,
            Some(95),
            None,
            Some(true),
            150,
        );
        let same_tick = tx(
            "same-tick",
            "attacker-3",
            10,
            3,
            "pool-1",
            TradeDirection::Sell,
            Some(95),
            None,
            Some(true),
            150,
        );
        let candidates =
            detect_front_run_candidates(&cfg, &[victim.clone(), suspect.clone(), other_pool, same_tick]);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].suspect_tx, "suspect");
    }

    #[test]
    fn confirm_front_run_detects_price_harm() {
        let cfg = DeaConfig::default();
        let victim = tx(
            "victim",
            "victim-actor",
            10,
            5,
            "pool-1",
            TradeDirection::Buy,
            Some(90),
            Some(100),
            Some(true),
            100,
        );
        let suspect = tx(
            "suspect",
            "attacker",
            11,
            4,
            "pool-1",
            TradeDirection::Sell,
            Some(95),
            None,
            Some(true),
            150,
        );
        let candidates = detect_front_run_candidates(&cfg, &[victim.clone(), suspect.clone()]);
        let findings = confirm_front_runs(&cfg, &[victim, suspect], &candidates);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].victim_harm, FrontRunHarmKind::WorsePrice);
        assert_eq!(findings[0].harm_metrics.price_degradation_bps, Some(1000));
    }

    #[test]
    fn confirm_front_run_detects_non_executable_victim() {
        let cfg = DeaConfig::default();
        let mut victim = tx(
            "victim",
            "victim-actor",
            10,
            0,
            "pool-1",
            TradeDirection::Buy,
            None,
            None,
            Some(false),
            0,
        );
        victim.ledger = None;
        let suspect = tx(
            "suspect",
            "attacker",
            11,
            4,
            "pool-1",
            TradeDirection::Sell,
            Some(95),
            None,
            Some(true),
            150,
        );
        let candidates = detect_front_run_candidates(&cfg, &[victim.clone(), suspect.clone()]);
        let findings = confirm_front_runs(&cfg, &[victim, suspect], &candidates);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].victim_harm, FrontRunHarmKind::BecameNonExecutable);
    }

    #[test]
    fn state_priority_only_is_capped_and_still_low_signal() {
        let mut cfg = DeaConfig::default();
        cfg.min_state_displacement_bps = 10;
        let victim = tx(
            "victim",
            "victim-actor",
            10,
            5,
            "pool-1",
            TradeDirection::Buy,
            Some(100),
            Some(100),
            Some(true),
            0,
        );
        let suspect = tx(
            "suspect",
            "attacker",
            11,
            4,
            "pool-1",
            TradeDirection::Sell,
            Some(95),
            None,
            Some(true),
            10,
        );
        let candidates = detect_front_run_candidates(&cfg, &[victim.clone(), suspect.clone()]);
        let findings = confirm_front_runs(&cfg, &[victim, suspect], &candidates);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].victim_harm, FrontRunHarmKind::StatePriorityOnly);
        assert!(findings[0].confidence.parse::<f64>().unwrap() < 0.80);
    }

    #[test]
    fn same_block_tx_index_controls_price_harm_ordering() {
        let cfg = DeaConfig::default();
        let mut victim = tx(
            "victim",
            "victim-actor",
            10,
            5,
            "pool-1",
            TradeDirection::Buy,
            Some(90),
            Some(100),
            Some(true),
            100,
        );
        victim.ledger.as_mut().unwrap().confirmed_tx_index = Some(2);
        let mut suspect = tx(
            "suspect",
            "attacker",
            11,
            5,
            "pool-1",
            TradeDirection::Sell,
            Some(95),
            None,
            Some(true),
            150,
        );
        suspect.ledger.as_mut().unwrap().confirmed_tx_index = Some(1);
        let candidates = detect_front_run_candidates(&cfg, &[victim.clone(), suspect.clone()]);
        let findings = confirm_front_runs(&cfg, &[victim, suspect], &candidates);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].victim_harm, FrontRunHarmKind::WorsePrice);
    }
}
