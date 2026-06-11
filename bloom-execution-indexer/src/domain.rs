use std::collections::BTreeMap;

pub type AssetId = String;
pub type BatcherId = String;
pub type PairId = String;
pub type TxHash = String;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderRef {
    pub tx_hash: TxHash,
    pub output_index: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OrderKind {
    Limit,
    Auction,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedOrder {
    pub order_ref: OrderRef,
    pub kind: OrderKind,
    pub pair: PairId,
    pub input_asset: AssetId,
    pub output_asset: AssetId,
    pub input_amount: u64,
    pub output_amount: Option<u64>,
    pub price_num: u64,
    pub price_denom: u64,
    pub permitted_executors: Vec<BatcherId>,
    pub created_slot: u64,
    pub created_time_ms: Option<u64>,
    pub status: OrderStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OrderStatus {
    Open,
    Executed {
        execution_tx: TxHash,
        attribution: ExecutionAttribution,
        slot: u64,
        time_ms: Option<u64>,
    },
    Cancelled {
        tx: TxHash,
        slot: u64,
        time_ms: Option<u64>,
    },
    UnknownSpent {
        tx: TxHash,
        slot: u64,
        time_ms: Option<u64>,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionAttribution {
    Single(BatcherId),
    Ambiguous(Vec<BatcherId>),
    Unknown,
    UnknownPermissionless,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRecord {
    pub tx_hash: TxHash,
    pub slot: u64,
    pub time_ms: Option<u64>,
    pub attribution: ExecutionAttribution,
    pub signer_batchers: Vec<BatcherId>,
    pub consumed_orders: Vec<OrderRef>,
    pub pair: Option<PairId>,
    pub classification: ExecutionClassification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionClassification {
    Executed,
    Cancelled,
    UnknownSpent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BatcherDiscoverySource {
    PermittedExecutor,
    ExecutionSigner,
    Both,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatcherProfile {
    pub pkh: BatcherId,
    pub first_seen_slot: u64,
    pub first_seen_ms: Option<u64>,
    pub last_seen_slot: u64,
    pub last_seen_ms: Option<u64>,
    pub source: BatcherDiscoverySource,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatcherMetrics {
    pub batcher: BatcherId,
    pub from_ms: Option<u64>,
    pub to_ms: Option<u64>,
    pub pair: Option<PairId>,
    pub eligible_orders: u64,
    pub executed_orders: u64,
    pub still_open_eligible_orders: u64,
    pub missed_eligible_orders: u64,
    pub capture_rate: Option<String>,
    pub median_response_ms: Option<u64>,
    pub p95_response_ms: Option<u64>,
    pub ambiguous_executions: u64,
    pub unknown_executions: u64,
    pub eligible_input_volume_by_asset: BTreeMap<AssetId, u128>,
    pub executed_input_volume_by_asset: BTreeMap<AssetId, u128>,
    pub executed_output_volume_by_asset: BTreeMap<AssetId, u128>,
    pub fees_collected_lovelace: Option<u128>,
}

pub fn asset_id(policy: &str, name_hex: &str) -> AssetId {
    if policy.is_empty() {
        "lovelace".to_string()
    } else {
        format!("{policy}.{name_hex}")
    }
}

pub fn pair_id(input: &str, output: &str) -> PairId {
    if input <= output {
        format!("{input}/{output}")
    } else {
        format!("{output}/{input}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_serialize_camel_case() {
        let metrics = BatcherMetrics {
            batcher: "abcd".to_string(),
            eligible_orders: 1,
            capture_rate: Some("1.0000".to_string()),
            ..Default::default()
        };

        let json = serde_json::to_value(metrics).unwrap();
        assert_eq!(json["eligibleOrders"], 1);
        assert_eq!(json["captureRate"], "1.0000");
    }
}
