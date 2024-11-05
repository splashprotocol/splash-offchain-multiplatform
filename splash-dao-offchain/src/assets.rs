use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("56d6abfe27017420f9683b17e0c3cd34dbb0cb134f28ac7a72563771").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
