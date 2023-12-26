use serde::Deserialize;

use spectrum_cardano_lib::OutputRef;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceSources {
    pub pool_v1_script: OutputRef,
    pub pool_v2_script: OutputRef,
    pub fee_switch_pool_script: OutputRef,
    pub fee_switch_pool_bidirectional_fee_script: OutputRef,
    pub swap_script: OutputRef,
    pub deposit_script: OutputRef,
    pub redeem_script: OutputRef,
}
