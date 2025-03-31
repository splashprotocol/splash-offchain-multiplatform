use cml_chain::plutus::{PlutusV2Script, PlutusV3Script};
use cml_crypto::RawBytesEncoding;
use uplc::tx::apply_params_to_script;

pub fn apply_params_validator_plutus_v2(
    params_pd: uplc::PlutusData,
    script: &str,
) -> cml_chain::plutus::PlutusV2Script {
    let params_bytes = uplc::plutus_data_to_bytes(&params_pd).unwrap();
    let script = PlutusV2Script::new(hex::decode(script).unwrap());

    let script_bytes = apply_params_to_script(&params_bytes, script.to_raw_bytes()).unwrap();

    cml_chain::plutus::PlutusV2Script::new(script_bytes)
}

pub fn apply_params_validator_plutus_v3(
    params_pd: uplc::PlutusData,
    script: &str,
) -> cml_chain::plutus::PlutusV3Script {
    let params_bytes = uplc::plutus_data_to_bytes(&params_pd).unwrap();
    let script = PlutusV3Script::new(hex::decode(script).unwrap());

    let script_bytes = apply_params_to_script(&params_bytes, script.to_raw_bytes()).unwrap();

    cml_chain::plutus::PlutusV3Script::new(script_bytes)
}
