use crate::orders::limit::LimitOrderValidation;
use spectrum_offchain_cardano::data::deposit::DepositOrderValidation;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::data::redeem::RedeemOrderValidation;

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationRules {
    pub limit_order: LimitOrderValidation,
    pub deposit_order: DepositOrderValidation,
    pub redeem_order: RedeemOrderValidation,
    pub pool: PoolValidation,
}
