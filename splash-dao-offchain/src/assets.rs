use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("cda0a6372c22af57510b43669ebce6115dbfa36d5d0fd175c4d2c4dd").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
