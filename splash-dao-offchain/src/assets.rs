use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("fc1b5c52084463dd11d6461cc94162df319b6f04f8377454d901213a").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
