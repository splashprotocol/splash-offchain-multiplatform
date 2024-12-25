use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("42d0f5972b7a2e334e7c8dacb6f4da8d15bbcce5e4152e8a935c9188").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
