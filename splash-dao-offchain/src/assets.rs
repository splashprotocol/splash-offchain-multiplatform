use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("bae4fc5ef13fcca83e64d8211bfe795dc791c502c642f90f3cb0deae").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
