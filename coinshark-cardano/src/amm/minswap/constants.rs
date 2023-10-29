use cml_chain::PolicyId;
use lazy_static::lazy_static;
use num_rational::Ratio;
use spectrum_cardano_lib::AssetName;

pub const POOL_NFT_POLICY_ID_HEX: &str = "0be55d262b29f564998ff81efe21bdc0022621c12f15af08d0f2ddb1";
pub const POOL_VALIDITY_POLICY_ID_HEX: &str = "13aa2accf2e1561723aa26871e071fdf32c867cff7e7d50ad470d62f";
pub const POOL_VALIDITY_ASSET_NAME_HEX: &str = "4d494e53574150";

lazy_static! {
    pub static ref POOL_FEE: Ratio<u64> = Ratio::new_raw(3, 1000);
    pub static ref POOL_NFT_POLICY_ID: PolicyId = PolicyId::from_hex(POOL_NFT_POLICY_ID_HEX).unwrap();
    pub static ref POOL_VALIDITY_POLICY_ID: PolicyId =
        PolicyId::from_hex(POOL_VALIDITY_POLICY_ID_HEX).unwrap();
    pub static ref POOL_VALIDITY_ASSET_NAME: AssetName =
        AssetName::from_hex(POOL_VALIDITY_ASSET_NAME_HEX).unwrap();
}
