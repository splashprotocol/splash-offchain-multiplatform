use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("4032bd3dc374fea51bf533b6e11712a92a86522d19ca36b2020d32d7").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
