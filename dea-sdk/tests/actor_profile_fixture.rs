use dea_sdk::{config::DeaConfig, domain::ReplayInput, replay, domain::RiskBand};

#[test]
fn actor_profile_fixture_requires_repeated_behavior_for_escalation() {
    let path = format!(
        "{}/tests/fixtures/actor_profile_basic.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(path).unwrap();
    let input: ReplayInput = serde_json::from_str(&raw).unwrap();
    let report = replay::assess(&DeaConfig::default(), &input);
    let repeated = report
        .actor_fairness_profiles
        .iter()
        .find(|p| p.actor_id == "repeat-attacker")
        .unwrap();
    let isolated = report
        .actor_fairness_profiles
        .iter()
        .find(|p| p.actor_id == "isolated-attacker")
        .unwrap();
    assert_eq!(repeated.risk_band, RiskBand::Medium);
    assert_eq!(isolated.risk_band, RiskBand::Low);
}
