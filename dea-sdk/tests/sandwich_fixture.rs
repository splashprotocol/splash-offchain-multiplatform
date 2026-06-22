use dea_sdk::{config::DeaConfig, domain::ReplayInput, replay};

#[test]
fn sandwich_fixture_produces_candidate_and_finding() {
    let path = format!(
        "{}/tests/fixtures/sandwich_basic.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(path).unwrap();
    let input: ReplayInput = serde_json::from_str(&raw).unwrap();
    let report = replay::assess(&DeaConfig::default(), &input);
    assert_eq!(report.sandwich_candidates.len(), 1);
    assert_eq!(report.sandwich_findings.len(), 1);
    assert_eq!(report.sandwich_findings[0].attacker_actor, "attacker-2");
    assert_eq!(report.actor_fairness_profiles[0].risk_band, dea_sdk::domain::RiskBand::Low);
}
