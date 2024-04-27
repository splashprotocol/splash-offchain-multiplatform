use crate::orders::limit::LimitOrderBounds;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::data::deposit::DepositOrderBounds;
use spectrum_offchain_cardano::data::pool::PoolBounds;
use spectrum_offchain_cardano::data::redeem::RedeemOrderBounds;

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub limit_order: LimitOrderBounds,
    pub deposit_order: DepositOrderBounds,
    pub redeem_order: RedeemOrderBounds,
    pub pool: PoolBounds
}
