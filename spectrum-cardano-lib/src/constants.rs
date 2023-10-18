use cml_chain::{Coin, PolicyId};
use lazy_static::lazy_static;

lazy_static! {
    pub static ref NATIVE_POLICY_ID: PolicyId = PolicyId::from([0u8; 28]);
}

pub const ZERO: Coin = 0;

pub const MIN_SAFE_ADA_VALUE: Coin = 3000000;

pub const MIN_TX_FEE: Coin = 300000;

pub const BABBAGE_ERA_ID: u16 = 5;
