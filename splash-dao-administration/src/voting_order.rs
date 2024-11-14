//! Code to create a voting_order for a user's voting_escrow
//!

use cml_chain::plutus::{PlutusData, PlutusScript, PlutusV2Script};
use cml_chain::Serialize;
use cml_crypto::{PrivateKey, RawBytesEncoding, ScriptHash};
use splash_dao_offchain::routines::inflation::actions::compute_farm_name;
use splash_dao_offchain::{
    constants::script_bytes::{MINT_IDENTIFIER_SCRIPT, VOTING_WITNESS_STUB},
    entities::{
        offchain::voting_order::{VotingOrder, VotingOrderId},
        onchain::{smart_farm::FarmId, voting_escrow::VotingEscrowId},
    },
};

pub fn create_voting_order(
    operator_sk: &PrivateKey,
    id: VotingOrderId,
    voting_power: u64,
    num_farms: u32,
) -> VotingOrder {
    let voting_witness_script =
        PlutusScript::PlutusV2(PlutusV2Script::new(hex::decode(VOTING_WITNESS_STUB).unwrap()));
    let redeemer = cml_chain::plutus::PlutusData::new_list(vec![]);
    let message = compute_voting_witness_message(voting_witness_script.hash(), redeemer.clone(), 0);
    let signature = operator_sk.sign(&message).to_raw_bytes().to_vec();

    // let mint_identifier_script = PlutusV2Script::new(hex::decode(MINT_IDENTIFIER_SCRIPT).unwrap());
    // let id = VotingOrderId::from((VotingEscrowId::from(mint_identifier_script.hash()), 0));

    VotingOrder {
        id,
        distribution: vec![
            (
                FarmId(spectrum_cardano_lib::AssetName::from(compute_farm_name(0))),
                voting_power,
            ),
            (
                FarmId(spectrum_cardano_lib::AssetName::from(compute_farm_name(1))),
                0,
            ),
        ],
        proof: signature,
        witness: voting_witness_script.hash(),
        witness_input: redeemer,
        version: 0,
    }
}

fn compute_voting_witness_message(
    witness: ScriptHash,
    witness_input: PlutusData,
    authenticated_version: u64,
) -> Vec<u8> {
    let mut bytes = witness.to_raw_bytes().to_vec();
    bytes.extend_from_slice(&witness_input.to_cbor_bytes());
    bytes.extend_from_slice(
        &PlutusData::new_integer(cml_chain::utils::BigInteger::from(authenticated_version)).to_cbor_bytes(),
    );
    cml_crypto::blake2b256(bytes.as_ref()).to_vec()
}
