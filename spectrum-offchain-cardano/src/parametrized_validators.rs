use cml_chain::{plutus::PlutusV2Script, PolicyId};
use cml_crypto::{RawBytesEncoding, ScriptHash};
use uplc::tx::apply_params_to_script;
use uplc_pallas_codec::utils::Bytes;
use uplc_pallas_traverse::ComputeHash;

fn apply_params_validator(params_pd: uplc::PlutusData, script: &str) -> ScriptHash {
    let params_bytes = uplc::plutus_data_to_bytes(&params_pd).unwrap();
    let script = PlutusV2Script::new(hex::decode(script).unwrap());

    let script_bytes = apply_params_to_script(&params_bytes, script.get()).unwrap();

    let script_hash =
        uplc_pallas_primitives::babbage::PlutusV2Script(Bytes::from(script_bytes)).compute_hash();

    PolicyId::from_raw_bytes(script_hash.as_slice()).unwrap()
}
