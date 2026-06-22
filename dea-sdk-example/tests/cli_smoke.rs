use std::process::Command;

#[test]
fn example_cli_prints_json_report() {
    let fixture = format!(
        "{}/../dea-sdk/tests/fixtures/front_run_basic.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = Command::new("cargo")
        .args([
            "run",
            "-q",
            "-p",
            "dea-sdk-example",
            "--",
            fixture.as_str(),
        ])
        .output()
        .expect("run example app");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["frontRunCandidates"].as_array().unwrap().len(), 1);
    assert_eq!(json["frontRunFindings"].as_array().unwrap().len(), 1);
    assert_eq!(json["frontRunFindings"][0]["victimHarm"], "worsePrice");
    assert_eq!(json["actorFairnessProfiles"].as_array().unwrap().len(), 1);
    assert_eq!(json["actorFairnessProfiles"][0]["actorId"], "attacker-1");
    assert_eq!(json["actorFairnessProfiles"][0]["riskBand"], "low");
}

#[test]
fn example_cli_prints_sandwich_report() {
    let fixture = format!(
        "{}/../dea-sdk/tests/fixtures/sandwich_basic.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = Command::new("cargo")
        .args([
            "run",
            "-q",
            "-p",
            "dea-sdk-example",
            "--",
            fixture.as_str(),
        ])
        .output()
        .expect("run example app");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["sandwichCandidates"].as_array().unwrap().len(), 1);
    assert_eq!(json["sandwichFindings"].as_array().unwrap().len(), 1);
    assert_eq!(json["sandwichFindings"][0]["attackerActor"], "attacker-2");
    assert_eq!(json["actorFairnessProfiles"][0]["riskBand"], "low");
}
