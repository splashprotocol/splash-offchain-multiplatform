use cml_chain::PolicyId;
use lazy_static::lazy_static;

use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{AssetClass, AssetName, Token};

#[derive(Clone, Serialize, Deserialize)]
pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token(Token(
        PolicyId::from_hex("93d35a410126ce11e3bd574f151e8314cbf27a34d0b683301111a668").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
