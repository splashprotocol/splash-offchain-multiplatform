use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("a272923aa1c92e6a458ee285e8ce0cc176f93aaef91565ebeb71707e").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
