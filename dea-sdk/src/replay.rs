use crate::{
    aggregation,
    attribution,
    config::DeaConfig,
    detectors::{front_run, sandwich},
    domain::{
        AssessmentReport, AssessmentWindow, ReplayInput, TransactionObservation, UnixMs,
    },
};

pub fn default_window(observations: &[TransactionObservation]) -> AssessmentWindow {
    let mut min_ms: Option<UnixMs> = None;
    let mut max_ms: Option<UnixMs> = None;
    for tx in observations {
        if let Some(m) = &tx.mempool {
            min_ms = Some(min_ms.map_or(m.first_seen_at, |v| v.min(m.first_seen_at)));
            let last = m.last_seen_at.unwrap_or(m.first_seen_at);
            max_ms = Some(max_ms.map_or(last, |v| v.max(last)));
        }
    }
    AssessmentWindow {
        from_ms: min_ms.unwrap_or(0),
        to_ms: max_ms.unwrap_or(0),
    }
}

pub fn assess(cfg: &DeaConfig, input: &ReplayInput) -> AssessmentReport {
    let mut observations = input.observations.clone();
    for tx in &mut observations {
        let attr = attribution::attribute_actor(tx, tx.steered_order.as_ref());
        tx.attribution = Some(attr);
    }
    let front_run_candidates = front_run::detect_front_run_candidates(cfg, &observations);
    let front_run_findings =
        front_run::confirm_front_runs(cfg, &observations, &front_run_candidates);
    let sandwich_candidates = sandwich::detect_sandwich_candidates(cfg, &observations);
    let sandwich_findings =
        sandwich::confirm_sandwiches(cfg, &observations, &sandwich_candidates);
    let assessment_window = input
        .assessment_window
        .clone()
        .unwrap_or_else(|| default_window(&observations));
    let actor_fairness_profiles = aggregation::aggregate_actor_fairness(
        cfg,
        &assessment_window,
        &front_run_candidates,
        &front_run_findings,
        &sandwich_candidates,
        &sandwich_findings,
    );
    AssessmentReport {
        assessment_window,
        front_run_candidates,
        front_run_findings,
        sandwich_candidates,
        sandwich_findings,
        actor_fairness_profiles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ReplayInput;

    #[test]
    fn empty_report_is_empty() {
        let report = assess(
            &DeaConfig::default(),
            &ReplayInput {
                assessment_window: None,
                observations: vec![],
            },
        );
        assert!(report.front_run_findings.is_empty());
        assert!(report.sandwich_findings.is_empty());
        assert!(report.actor_fairness_profiles.is_empty());
    }
}

