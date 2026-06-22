use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskWeights {
    pub confirmed_event_rate: String,
    pub candidate_event_rate: String,
    pub weighted_severity_rate: String,
    pub repeat_offense_factor: String,
    pub victim_diversity_factor: String,
    pub persistence_factor: String,
    pub market_concentration_factor: String,
}

impl Default for RiskWeights {
    fn default() -> Self {
        Self {
            confirmed_event_rate: "0.3000".to_string(),
            candidate_event_rate: "0.0500".to_string(),
            weighted_severity_rate: "0.2000".to_string(),
            repeat_offense_factor: "0.1500".to_string(),
            victim_diversity_factor: "0.1000".to_string(),
            persistence_factor: "0.1000".to_string(),
            market_concentration_factor: "0.1000".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeaConfig {
    pub front_run_window_ms: u64,
    pub min_price_impact_bps: u64,
    pub min_victim_harm_bps: u64,
    pub min_state_displacement_bps: u64,
    pub max_same_tick_ambiguity: u64,
    pub min_suspect_input_amount: u128,
    pub min_actor_attribution_confidence: crate::domain::AttributionConfidence,
    pub min_unwind_ratio_bps: u64,
    pub repeat_offense_denominator: u64,
    pub min_opportunity_count: u64,
    pub min_high_confidence_findings: u64,
    pub min_confirmed_findings_for_medium: u64,
    pub candidate_support_threshold: u64,
    pub min_distinct_victims: u64,
    pub min_distinct_days: u64,
    pub persistence_window_denominator: u64,
    pub decay_half_life_ms: Option<u64>,
    pub risk_weights: RiskWeights,
}

impl Default for DeaConfig {
    fn default() -> Self {
        Self {
            front_run_window_ms: 10_000,
            min_price_impact_bps: 50,
            min_victim_harm_bps: 25,
            min_state_displacement_bps: 50,
            max_same_tick_ambiguity: 0,
            min_suspect_input_amount: 1,
            min_actor_attribution_confidence: crate::domain::AttributionConfidence::Medium,
            min_unwind_ratio_bps: 5_000,
            repeat_offense_denominator: 3,
            min_opportunity_count: 3,
            min_high_confidence_findings: 2,
            min_confirmed_findings_for_medium: 2,
            candidate_support_threshold: 2,
            min_distinct_victims: 2,
            min_distinct_days: 2,
            persistence_window_denominator: 2,
            decay_half_life_ms: None,
            risk_weights: RiskWeights::default(),
        }
    }
}

