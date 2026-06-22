use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type ActorId = String;
pub type AssetId = String;
pub type PoolId = String;
pub type PairId = String;
pub type TxHash = String;
pub type UnixMs = u64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfidenceBand {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AttributionConfidence {
    High,
    Medium,
    Low,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskBand {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceKind {
    MempoolFirstSeen,
    LedgerConfirmation,
    PoolStateDelta,
    CounterfactualSimulation,
    ActorAttribution,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRef {
    pub tx_hash: TxHash,
    pub kind: EvidenceKind,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessmentWindow {
    pub from_ms: UnixMs,
    pub to_ms: UnixMs,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderRef {
    pub tx_hash: TxHash,
    pub output_index: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignerCredential {
    pub pkh: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteeredOrderContext {
    pub order_ref: Option<OrderRef>,
    pub permitted_executors: Vec<ActorId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributionRecord {
    pub actor_id: Option<ActorId>,
    pub confidence: AttributionConfidence,
    pub matched_permitted_executor: Option<ActorId>,
    pub signer_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InteractionRole {
    OrderCreation,
    OrderExecution,
    DirectSwap,
    Cancel,
    LiquidityChange,
    Admin,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TradeDirection {
    Buy,
    Sell,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketTouch {
    pub pool_id: PoolId,
    pub pair_id: PairId,
    pub role: InteractionRole,
    pub direction: TradeDirection,
    pub base_asset: AssetId,
    pub quote_asset: AssetId,
    pub input_amount: u128,
    pub output_amount: Option<u128>,
    pub output_loss_asset: Option<AssetId>,
    pub output_loss_amount: Option<u128>,
    pub counterfactual_output_amount: Option<u128>,
    pub executable_after: Option<bool>,
    pub state_displacement_bps: u64,
    pub net_base_flow: i128,
    pub net_quote_flow: i128,
    pub depends_on_txs: Vec<TxHash>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MempoolObservation {
    pub first_seen_at: UnixMs,
    pub last_seen_at: Option<UnixMs>,
    pub mempool_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerObservation {
    pub confirmed_slot: Option<u64>,
    pub confirmed_block_hash: Option<String>,
    pub confirmed_tx_index: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionObservation {
    pub tx_hash: TxHash,
    pub mempool: Option<MempoolObservation>,
    pub ledger: Option<LedgerObservation>,
    pub signers: Vec<SignerCredential>,
    pub touches: Vec<MarketTouch>,
    pub steered_order: Option<SteeredOrderContext>,
    pub attribution: Option<AttributionRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationBatch {
    pub transactions: Vec<TransactionObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayInput {
    pub assessment_window: Option<AssessmentWindow>,
    pub observations: Vec<TransactionObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontRunCandidate {
    pub victim_tx: TxHash,
    pub suspect_tx: TxHash,
    pub victim_first_seen_at: UnixMs,
    pub pool_id: PoolId,
    pub pair_id: PairId,
    pub first_seen_delta_ms: u64,
    pub victim_actor: Option<ActorId>,
    pub suspect_actor: Option<ActorId>,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontRunHarmKind {
    WorsePrice,
    BecameNonExecutable,
    StatePriorityOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontRunHarmMetrics {
    pub price_degradation_bps: Option<u64>,
    pub execution_slot_delta: Option<i64>,
    pub counterfactual_computable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontRunFinding {
    pub victim_tx: TxHash,
    pub suspect_tx: TxHash,
    pub victim_first_seen_at: UnixMs,
    pub suspect_first_seen_at: UnixMs,
    pub first_seen_delta_ms: u64,
    pub victim_confirmed_slot: Option<u64>,
    pub suspect_confirmed_slot: Option<u64>,
    pub pair_ids: Vec<PairId>,
    pub pool_ids: Vec<PoolId>,
    pub victim_harm: FrontRunHarmKind,
    pub harm_metrics: FrontRunHarmMetrics,
    pub suspect_actor: Option<ActorId>,
    pub confidence: String,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandwichCandidate {
    pub pre_tx: TxHash,
    pub victim_tx: TxHash,
    pub post_tx: TxHash,
    pub victim_first_seen_at: UnixMs,
    pub attacker_actor: Option<ActorId>,
    pub pool_id: PoolId,
    pub pair_id: PairId,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VictimHarmMetrics {
    pub price_degradation_bps: Option<u64>,
    pub output_loss_asset: Option<AssetId>,
    pub output_loss_amount: Option<u128>,
    pub baseline_computable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttackerBenefitMetrics {
    pub quote_asset: Option<AssetId>,
    pub estimated_gross_profit: Option<i128>,
    pub unwind_ratio_bps: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandwichFinding {
    pub pre_tx: TxHash,
    pub victim_tx: TxHash,
    pub post_tx: TxHash,
    pub attacker_actor: ActorId,
    pub pair_ids: Vec<PairId>,
    pub pool_ids: Vec<PoolId>,
    pub victim_harm_metrics: VictimHarmMetrics,
    pub attacker_benefit_metrics: AttackerBenefitMetrics,
    pub confidence: String,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorFairnessProfile {
    pub actor_id: ActorId,
    pub assessment_window: AssessmentWindow,
    pub front_run_candidate_count: u64,
    pub sandwich_candidate_count: u64,
    pub front_run_count: u64,
    pub sandwich_count: u64,
    pub distinct_victim_count: u64,
    pub distinct_day_count: u64,
    pub distinct_pool_count: u64,
    pub observed_actor_opportunities: u64,
    pub candidate_event_count: u64,
    pub confirmed_event_count: u64,
    pub high_confidence_findings: u64,
    pub weighted_findings: String,
    pub confirmed_event_rate: String,
    pub candidate_event_rate: String,
    pub weighted_severity_rate: String,
    pub repeat_offense_factor: String,
    pub victim_diversity_factor: String,
    pub persistence_factor: String,
    pub market_concentration_factor: String,
    pub risk_score: String,
    pub risk_band: RiskBand,
    pub escalation_reasons: Vec<String>,
    pub top_evidence_refs: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessmentReport {
    pub assessment_window: AssessmentWindow,
    pub front_run_candidates: Vec<FrontRunCandidate>,
    pub front_run_findings: Vec<FrontRunFinding>,
    pub sandwich_candidates: Vec<SandwichCandidate>,
    pub sandwich_findings: Vec<SandwichFinding>,
    pub actor_fairness_profiles: Vec<ActorFairnessProfile>,
}

pub fn pair_id(asset_a: &str, asset_b: &str) -> PairId {
    if asset_a <= asset_b {
        format!("{asset_a}/{asset_b}")
    } else {
        format!("{asset_b}/{asset_a}")
    }
}

pub fn fixed_4(value: f64) -> String {
    format!("{value:.4}")
}

pub fn day_bucket(ms: UnixMs) -> u64 {
    ms / 86_400_000
}

pub fn parse_fixed_4(value: &str) -> f64 {
    value.parse::<f64>().unwrap_or(0.0)
}

pub fn top_pair_count<'a, I>(pairs: I) -> u64
where
    I: Iterator<Item = &'a PairId>,
{
    let mut counts: BTreeMap<&PairId, u64> = BTreeMap::new();
    for pair in pairs {
        *counts.entry(pair).or_default() += 1;
    }
    counts.values().copied().max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_id_is_canonical() {
        assert_eq!(pair_id("bbb", "aaa"), "aaa/bbb");
    }

    #[test]
    fn serialize_actor_profile_camel_case() {
        let profile = ActorFairnessProfile {
            actor_id: "actor".into(),
            assessment_window: AssessmentWindow { from_ms: 1, to_ms: 2 },
            front_run_candidate_count: 1,
            sandwich_candidate_count: 0,
            front_run_count: 0,
            sandwich_count: 0,
            distinct_victim_count: 1,
            distinct_day_count: 1,
            distinct_pool_count: 1,
            observed_actor_opportunities: 1,
            candidate_event_count: 1,
            confirmed_event_count: 0,
            high_confidence_findings: 0,
            weighted_findings: "0.0500".into(),
            confirmed_event_rate: "0.0000".into(),
            candidate_event_rate: "1.0000".into(),
            weighted_severity_rate: "0.0500".into(),
            repeat_offense_factor: "0.0000".into(),
            victim_diversity_factor: "0.0000".into(),
            persistence_factor: "0.0000".into(),
            market_concentration_factor: "1.0000".into(),
            risk_score: "0.0500".into(),
            risk_band: RiskBand::Low,
            escalation_reasons: vec!["observe only".into()],
            top_evidence_refs: vec![],
        };

        let json = serde_json::to_value(profile).unwrap();
        assert_eq!(json["riskBand"], "low");
        assert_eq!(json["candidateEventCount"], 1);
    }
}
