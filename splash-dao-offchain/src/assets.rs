use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("6a20402560dd195a696155db039bfb8326bda96e93e890cd89321571").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
