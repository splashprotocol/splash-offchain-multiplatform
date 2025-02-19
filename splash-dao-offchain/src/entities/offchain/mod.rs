use cml_chain::plutus::PlutusData;
use cml_crypto::RawBytesEncoding;
use cml_crypto::ScriptHash;
pub mod extend_voting_escrow_order;
pub mod voting_order;

pub fn compute_voting_escrow_witness_message(
    witness: ScriptHash,
    witness_input: String,
    authenticated_version: u64,
) -> Result<Vec<u8>, ()> {
    use cml_chain::Serialize;
    let mut bytes = witness.to_raw_bytes().to_vec();
    let witness_input_cbor = hex::decode(witness_input).map_err(|_| ())?;
    bytes.extend_from_slice(&witness_input_cbor);
    bytes.extend_from_slice(
        &PlutusData::new_integer(cml_chain::utils::BigInteger::from(authenticated_version)).to_cbor_bytes(),
    );
    Ok(cml_crypto::blake2b256(bytes.as_ref()).to_vec())
}
