use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("54d633e284a31e7685d52e140458f1ce2cdc09ea77799de01cc45715").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
