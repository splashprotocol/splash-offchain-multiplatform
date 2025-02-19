use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("f05256a358d0b0594e523283a67b04147abc9745c20e64d2b2bcbe9f").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
