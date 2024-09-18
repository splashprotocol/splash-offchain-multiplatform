use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("f8b31f96a1925a6bed07c0b16ca39efebd1cbd786057216c4b5eae5b").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
