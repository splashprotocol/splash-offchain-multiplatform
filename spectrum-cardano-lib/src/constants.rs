use cml_chain::{Coin, PolicyId};
use lazy_static::lazy_static;
use std::time::Duration;

lazy_static! {
    pub static ref NATIVE_POLICY_ID: PolicyId = PolicyId::from([0u8; 28]);
}

pub const ZERO: Coin = 0;

pub const MIN_TX_FEE: Coin = 300000;

pub const BABBAGE_ERA_ID: u16 = 5;
pub const CONWAY_ERA_ID: u16 = 6;

pub const SAFE_BLOCK_TIME: Duration = Duration::from_secs(20 * 4);

pub const ED25519_PUB_KEY_LENGTH: usize = 32;

pub const CURRENCY_SYMBOL_HEX_STRING_LENGTH: usize = 56;
