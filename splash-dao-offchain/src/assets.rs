use cml_chain::PolicyId;
use lazy_static::lazy_static;

use spectrum_cardano_lib::{AssetClass, AssetName};

pub struct Splash;

lazy_static! {
    pub static ref SPLASH_AC: AssetClass = AssetClass::Token((
        PolicyId::from_hex("").unwrap(),
        AssetName::utf8_unsafe("SPLASH".to_string())
    ));
}
