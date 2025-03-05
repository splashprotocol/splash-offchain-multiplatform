use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("8a44198740ec8629a3442cc13d2329706b0202c39cbe88b0ac4ed825").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
