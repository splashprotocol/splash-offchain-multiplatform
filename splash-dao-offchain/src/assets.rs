use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("e8cfc20dac8daff83e8382b91fe0dd1a0acb47f3161e1a286da88321").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
