use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("9b9be5917c082184cf40363cf6209a1c0cf4296d6358b471684eb9d9").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
