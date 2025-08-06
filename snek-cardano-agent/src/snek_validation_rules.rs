use bloom_offchain_cardano::orders::instant::InstantOrderValidation;
use spectrum_offchain_cardano::data::pool::PoolValidation;

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnekValidationRules {
    pub instant_order: InstantOrderValidation,
    pub pool: PoolValidation,
}
