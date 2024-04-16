use crate::orders::limit::LimitOrderBounds;

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub limit_order: LimitOrderBounds,
}
