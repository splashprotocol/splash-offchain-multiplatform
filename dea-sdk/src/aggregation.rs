use crate::config::DeaConfig;
use crate::domain::{
    day_bucket, fixed_4, parse_fixed_4, top_pair_count, ActorFairnessProfile, AssessmentWindow,
    EvidenceRef, FrontRunCandidate, FrontRunFinding, RiskBand, SandwichCandidate, SandwichFinding,
};
use std::collections::{BTreeMap, BTreeSet};

fn rate(n: u64, d: u64) -> f64 {
    if d == 0 {
        0.0
    } else {
        n as f64 / d as f64
    }
}

fn weight_sum(findings: &[f64]) -> f64 {
    findings.iter().sum()
}

fn risk_band(
    cfg: &DeaConfig,
    confirmed_event_count: u64,
    candidate_event_count: u64,
    distinct_victim_count: u64,
    distinct_day_count: u64,
    opportunities: u64,
    high_confidence_findings: u64,
    score: f64,
) -> RiskBand {
    let medium_entry = confirmed_event_count >= cfg.min_confirmed_findings_for_medium
        || (confirmed_event_count >= 1
            && distinct_victim_count >= 2
            && (distinct_day_count >= 2 || candidate_event_count >= cfg.candidate_support_threshold));
    let high_entry = opportunities >= cfg.min_opportunity_count
        && high_confidence_findings >= cfg.min_high_confidence_findings
        && distinct_victim_count >= cfg.min_distinct_victims
        && distinct_day_count >= cfg.min_distinct_days;
    if high_entry && score >= 0.50 {
        RiskBand::High
    } else if medium_entry && score >= 0.20 {
        RiskBand::Medium
    } else {
        RiskBand::Low
    }
}

pub fn aggregate_actor_fairness(
    cfg: &DeaConfig,
    window: &AssessmentWindow,
    front_run_candidates: &[FrontRunCandidate],
    front_run_findings: &[FrontRunFinding],
    sandwich_candidates: &[SandwichCandidate],
    sandwich_findings: &[SandwichFinding],
) -> Vec<ActorFairnessProfile> {
    #[derive(Default)]
    struct State {
        fr_candidates: u64,
        sw_candidates: u64,
        fr_findings: u64,
        sw_findings: u64,
        evidence: Vec<EvidenceRef>,
        victims: BTreeSet<String>,
        days: BTreeSet<u64>,
        pools: BTreeSet<String>,
        opportunities: BTreeSet<String>,
        candidate_opportunities: BTreeSet<String>,
        confirmed_opportunities: BTreeSet<String>,
        pair_ids: Vec<String>,
        confirmed_pair_ids: Vec<String>,
        weighted_findings: Vec<f64>,
        high_confidence: u64,
    }

    let mut states: BTreeMap<String, State> = BTreeMap::new();
    for c in front_run_candidates {
        if c.victim_first_seen_at < window.from_ms || c.victim_first_seen_at > window.to_ms {
            continue;
        }
        if let Some(actor) = &c.suspect_actor {
            let s = states.entry(actor.clone()).or_default();
            s.fr_candidates += 1;
            s.victims.insert(c.victim_tx.clone());
            s.pools.insert(c.pool_id.clone());
            s.pair_ids.push(c.pair_id.clone());
            s.opportunities.insert(c.victim_tx.clone());
            s.candidate_opportunities.insert(c.victim_tx.clone());
            s.days.insert(day_bucket(c.victim_first_seen_at));
            s.evidence.extend(c.evidence_refs.clone());
        }
    }
    for c in sandwich_candidates {
        if c.victim_first_seen_at < window.from_ms || c.victim_first_seen_at > window.to_ms {
            continue;
        }
        if let Some(actor) = &c.attacker_actor {
            let s = states.entry(actor.clone()).or_default();
            s.sw_candidates += 1;
            s.victims.insert(c.victim_tx.clone());
            s.pools.insert(c.pool_id.clone());
            s.pair_ids.push(c.pair_id.clone());
            s.opportunities.insert(c.victim_tx.clone());
            s.candidate_opportunities.insert(c.victim_tx.clone());
            s.days.insert(day_bucket(c.victim_first_seen_at));
            s.evidence.extend(c.evidence_refs.clone());
        }
    }
    for f in front_run_findings {
        if f.victim_first_seen_at < window.from_ms || f.victim_first_seen_at > window.to_ms {
            continue;
        }
        if let Some(actor) = &f.suspect_actor {
            let s = states.entry(actor.clone()).or_default();
            s.fr_findings += 1;
            s.victims.insert(f.victim_tx.clone());
            s.pools.extend(f.pool_ids.iter().cloned());
            s.pair_ids.extend(f.pair_ids.iter().cloned());
            s.opportunities.insert(f.victim_tx.clone());
            s.confirmed_opportunities.insert(f.victim_tx.clone());
            s.evidence.extend(f.evidence_refs.clone());
            if f.victim_first_seen_at >= window.from_ms && f.victim_first_seen_at <= window.to_ms {
                s.days.insert(day_bucket(f.victim_first_seen_at));
            }
            s.confirmed_pair_ids.extend(f.pair_ids.iter().cloned());
            let conf = parse_fixed_4(&f.confidence);
            s.weighted_findings.push(conf);
            if conf >= 0.80 {
                s.high_confidence += 1;
            }
        }
    }
    for f in sandwich_findings {
        let victim_first_seen_at = sandwich_candidates
            .iter()
            .find(|c| c.pre_tx == f.pre_tx && c.victim_tx == f.victim_tx && c.post_tx == f.post_tx)
            .map(|c| c.victim_first_seen_at);
        let Some(victim_first_seen_at) = victim_first_seen_at else {
            continue;
        };
        if victim_first_seen_at < window.from_ms || victim_first_seen_at > window.to_ms {
            continue;
        }
        let s = states.entry(f.attacker_actor.clone()).or_default();
        s.sw_findings += 1;
        s.victims.insert(f.victim_tx.clone());
        s.pools.extend(f.pool_ids.iter().cloned());
        s.pair_ids.extend(f.pair_ids.iter().cloned());
        s.opportunities.insert(f.victim_tx.clone());
        s.confirmed_opportunities.insert(f.victim_tx.clone());
        s.evidence.extend(f.evidence_refs.clone());
        s.days.insert(day_bucket(victim_first_seen_at));
        s.confirmed_pair_ids.extend(f.pair_ids.iter().cloned());
        let conf = parse_fixed_4(&f.confidence);
        s.weighted_findings.push(conf);
        if conf >= 0.80 {
            s.high_confidence += 1;
        }
    }

    let weights = &cfg.risk_weights;
    let w_confirmed = parse_fixed_4(&weights.confirmed_event_rate);
    let w_candidate = parse_fixed_4(&weights.candidate_event_rate);
    let w_severity = parse_fixed_4(&weights.weighted_severity_rate);
    let w_repeat = parse_fixed_4(&weights.repeat_offense_factor);
    let w_victim = parse_fixed_4(&weights.victim_diversity_factor);
    let w_persistence = parse_fixed_4(&weights.persistence_factor);
    let w_market = parse_fixed_4(&weights.market_concentration_factor);

    let mut profiles = Vec::new();
    for (actor, state) in states {
        let candidate_event_count = state.candidate_opportunities.len() as u64;
        let confirmed_event_count = state.confirmed_opportunities.len() as u64;
        let opportunities = state.opportunities.len() as u64;
        let distinct_victim_count = state.victims.len() as u64;
        let distinct_day_count = state.days.len() as u64;
        let distinct_pool_count = state.pools.len() as u64;
        let confirmed_event_rate = rate(confirmed_event_count, opportunities);
        let candidate_event_rate = rate(candidate_event_count, opportunities);
        let weighted_findings = weight_sum(&state.weighted_findings);
        let weighted_severity_rate = if opportunities == 0 {
            0.0
        } else {
            weighted_findings / opportunities as f64
        };
        let repeat_offense_factor =
            (state.high_confidence as f64 / cfg.repeat_offense_denominator as f64).min(1.0);
        let victim_diversity_factor = if confirmed_event_count == 0 {
            0.0
        } else {
            distinct_victim_count as f64 / confirmed_event_count as f64
        };
        let persistence_factor =
            (distinct_day_count as f64 / cfg.persistence_window_denominator as f64).min(1.0);
        let market_concentration_factor = if confirmed_event_count == 0 {
            0.0
        } else {
            top_pair_count(state.confirmed_pair_ids.iter()) as f64
                / (state.confirmed_pair_ids.len() as f64).max(1.0)
        };
        let risk_score = w_confirmed * confirmed_event_rate
            + w_candidate * candidate_event_rate
            + w_severity * weighted_severity_rate
            + w_repeat * repeat_offense_factor
            + w_victim * victim_diversity_factor
            + w_persistence * persistence_factor
            + w_market * market_concentration_factor;
        let band = risk_band(
            cfg,
            confirmed_event_count,
            candidate_event_count,
            distinct_victim_count,
            distinct_day_count,
            opportunities,
            state.high_confidence,
            risk_score,
        );
        let mut reasons = vec!["accumulate evidence".to_string()];
        if matches!(band, RiskBand::Medium | RiskBand::High) {
            reasons.push("repeated suspicious behavior".to_string());
        }
        if band == RiskBand::High {
            reasons.push("high-risk thresholds satisfied".to_string());
        }
        profiles.push(ActorFairnessProfile {
            actor_id: actor,
            assessment_window: window.clone(),
            front_run_candidate_count: state.fr_candidates,
            sandwich_candidate_count: state.sw_candidates,
            front_run_count: state.fr_findings,
            sandwich_count: state.sw_findings,
            distinct_victim_count,
            distinct_day_count,
            distinct_pool_count,
            observed_actor_opportunities: opportunities,
            candidate_event_count,
            confirmed_event_count,
            high_confidence_findings: state.high_confidence,
            weighted_findings: fixed_4(weighted_findings),
            confirmed_event_rate: fixed_4(confirmed_event_rate),
            candidate_event_rate: fixed_4(candidate_event_rate),
            weighted_severity_rate: fixed_4(weighted_severity_rate),
            repeat_offense_factor: fixed_4(repeat_offense_factor),
            victim_diversity_factor: fixed_4(victim_diversity_factor),
            persistence_factor: fixed_4(persistence_factor),
            market_concentration_factor: fixed_4(market_concentration_factor),
            risk_score: fixed_4(risk_score),
            risk_band: band,
            escalation_reasons: reasons,
            top_evidence_refs: state.evidence.into_iter().take(5).collect(),
        });
    }
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AssessmentWindow, FrontRunCandidate, FrontRunFinding, FrontRunHarmKind, FrontRunHarmMetrics};

    fn fr_candidate(victim: &str, actor: &str, delta: u64) -> FrontRunCandidate {
        FrontRunCandidate {
            victim_tx: victim.into(),
            suspect_tx: format!("suspect-{victim}"),
            victim_first_seen_at: 0,
            pool_id: "pool".into(),
            pair_id: "a/b".into(),
            first_seen_delta_ms: delta,
            victim_actor: None,
            suspect_actor: Some(actor.into()),
            evidence_refs: vec![],
        }
    }

    fn fr_finding(victim: &str, actor: &str, conf: &str, first_seen: u64) -> FrontRunFinding {
        FrontRunFinding {
            victim_tx: victim.into(),
            suspect_tx: format!("suspect-{victim}"),
            victim_first_seen_at: first_seen,
            suspect_first_seen_at: first_seen + 1,
            first_seen_delta_ms: 1,
            victim_confirmed_slot: Some(10),
            suspect_confirmed_slot: Some(9),
            pair_ids: vec!["a/b".into()],
            pool_ids: vec!["pool".into()],
            victim_harm: FrontRunHarmKind::WorsePrice,
            harm_metrics: FrontRunHarmMetrics {
                price_degradation_bps: Some(100),
                execution_slot_delta: Some(-1),
                counterfactual_computable: true,
            },
            suspect_actor: Some(actor.into()),
            confidence: conf.into(),
            evidence_refs: vec![],
        }
    }

    #[test]
    fn isolated_candidate_stays_low_risk() {
        let cfg = DeaConfig::default();
        let profiles = aggregate_actor_fairness(
            &cfg,
            &AssessmentWindow { from_ms: 0, to_ms: 200_000_000 },
            &[fr_candidate("victim-1", "attacker", 1)],
            &[],
            &[],
            &[],
        );
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].risk_band, RiskBand::Low);
    }

    #[test]
    fn isolated_confirmed_finding_stays_low_risk() {
        let cfg = DeaConfig::default();
        let profiles = aggregate_actor_fairness(
            &cfg,
            &AssessmentWindow { from_ms: 0, to_ms: 200_000_000 },
            &[fr_candidate("victim-1", "attacker", 1)],
            &[fr_finding("victim-1", "attacker", "0.8500", 10)],
            &[],
            &[],
        );
        assert_eq!(profiles[0].risk_band, RiskBand::Low);
    }

    #[test]
    fn repeated_behavior_across_victims_and_days_escalates() {
        let cfg = DeaConfig::default();
        let profiles = aggregate_actor_fairness(
            &cfg,
            &AssessmentWindow { from_ms: 0, to_ms: 300_000_000 },
            &[
                fr_candidate("victim-1", "attacker", 1),
                fr_candidate("victim-2", "attacker", 1),
                fr_candidate("victim-3", "attacker", 1),
            ],
            &[
                fr_finding("victim-1", "attacker", "0.8500", 10),
                fr_finding("victim-2", "attacker", "0.8500", 100_000_000),
            ],
            &[],
            &[],
        );
        assert_eq!(profiles[0].risk_band, RiskBand::High);
    }
}
