use bloom_offchain_cardano::orders::limit::LimitOrderValidation;
use spectrum_offchain_cardano::data::pool::PoolValidation;

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnekValidationRules {
    pub limit_order: LimitOrderValidation,
    pub pool: PoolValidation,
}
