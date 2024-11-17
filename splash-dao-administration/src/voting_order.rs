//! Code to create a voting_order for a user's voting_escrow
//!

use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::{ConstrPlutusData, PlutusData, PlutusScript, PlutusV2Script};
use cml_chain::utils::BigInteger;
use cml_chain::{LenEncoding, PolicyId, Serialize};
use cml_crypto::{PrivateKey, RawBytesEncoding, ScriptHash};
use splash_dao_offchain::constants::script_bytes::VOTING_WITNESS;
use splash_dao_offchain::entities::onchain::weighting_poll::distribution_to_plutus_data;
use splash_dao_offchain::routines::inflation::actions::{compute_epoch_asset_name, compute_farm_name};
use splash_dao_offchain::{
    constants::script_bytes::{MINT_IDENTIFIER_SCRIPT, VOTING_WITNESS_STUB},
    entities::{
        offchain::voting_order::{VotingOrder, VotingOrderId},
        onchain::{smart_farm::FarmId, voting_escrow::VotingEscrowId},
    },
};
use uplc_pallas_primitives::Fragment;

pub fn create_voting_order(
    operator_sk: &PrivateKey,
    id: VotingOrderId,
    voting_power: u64,
    wpoll_policy_id: PolicyId,
    num_farms: u32,
) -> VotingOrder {
    let voting_witness_script =
        PlutusScript::PlutusV2(PlutusV2Script::new(hex::decode(VOTING_WITNESS).unwrap()));
    let distribution = vec![
        (
            FarmId(spectrum_cardano_lib::AssetName::from(compute_farm_name(0))),
            voting_power,
        ),
        (
            FarmId(spectrum_cardano_lib::AssetName::from(compute_farm_name(1))),
            0,
        ),
    ];
    let redeemer = make_witness_redeemer(&distribution, wpoll_policy_id, 0);
    let redeemer_pallas = make_pallas_redeemer(wpoll_policy_id, 0);
    let message = compute_voting_witness_message(voting_witness_script.hash(), redeemer_pallas, 0);
    println!("MESSAGE: {}", hex::encode(&message));
    let signature = operator_sk.sign(&message).to_raw_bytes().to_vec();

    VotingOrder {
        id,
        distribution,
        proof: signature,
        witness: voting_witness_script.hash(),
        witness_input: redeemer,
        version: 0,
    }
}

fn compute_voting_witness_message(
    witness: ScriptHash,
    witness_input: uplc::PlutusData,
    authenticated_version: u64,
) -> Vec<u8> {
    let mut bytes = witness.to_raw_bytes().to_vec();
    let witness_input_cbor = witness_input.encode_fragment().unwrap();
    bytes.extend_from_slice(&witness_input_cbor);
    bytes.extend_from_slice(
        &PlutusData::new_integer(cml_chain::utils::BigInteger::from(authenticated_version)).to_cbor_bytes(),
    );
    cml_crypto::blake2b256(bytes.as_ref()).to_vec()
}

fn make_witness_redeemer(
    distribution: &[(FarmId, u64)],
    wpoll_policy_id: ScriptHash,
    epoch: u32,
) -> PlutusData {
    let wpoll_auth_token_name = compute_epoch_asset_name(epoch);
    println!("wpoll_auth_name: {}", wpoll_auth_token_name.to_raw_hex());

    let asset_pd = make_constr_pd_indefinite_arr(vec![
        PlutusData::new_bytes(wpoll_policy_id.to_raw_bytes().to_vec()),
        PlutusData::new_bytes(wpoll_auth_token_name.to_raw_bytes().to_vec()),
    ]);

    let farm_0_name = compute_farm_name(0);

    // let distribution_pd = distribution_to_plutus_data(distribution);
    let distribution_pd = PlutusData::List {
        list: vec![make_constr_pd_indefinite_arr(vec![
            PlutusData::new_bytes(farm_0_name.inner),
            PlutusData::new_integer(BigInteger::from(-20_i64)),
        ])],
        list_encoding: LenEncoding::Indefinite,
    };
    make_constr_pd_indefinite_arr(vec![asset_pd, distribution_pd])
}

/// There are differences in how Aiken and CML serialise PlutusData. Here we'll form the redeemer
/// using the `PlutusData` representation from the Pallas crates, since it is used internally
/// by Aiken.
fn make_pallas_redeemer(wpoll_policy_id: ScriptHash, epoch: u32) -> uplc::PlutusData {
    let wpoll_auth_token_name = compute_epoch_asset_name(epoch);

    let to_plutus_bytes =
        |bytes: Vec<u8>| uplc::PlutusData::BoundedBytes(uplc_pallas_codec::utils::PlutusBytes::from(bytes));
    let to_plutus_cstr = |fields: Vec<uplc::PlutusData>| {
        uplc::PlutusData::Constr(uplc::Constr {
            tag: 121,
            any_constructor: None,
            fields,
        })
    };
    let asset_pd = to_plutus_cstr(vec![
        to_plutus_bytes(wpoll_policy_id.to_raw_bytes().to_vec()),
        to_plutus_bytes(wpoll_auth_token_name.to_raw_bytes().to_vec()),
    ]);

    let farm_0_name = compute_farm_name(0);
    let distribution_pd = uplc::PlutusData::Array(vec![to_plutus_cstr(vec![
        to_plutus_bytes(farm_0_name.inner),
        uplc::PlutusData::BigInt(uplc::BigInt::Int(uplc_pallas_codec::utils::Int::from(-20_i64))),
    ])]);
    to_plutus_cstr(vec![asset_pd, distribution_pd])
}

fn make_constr_pd_indefinite_arr(fields: Vec<PlutusData>) -> PlutusData {
    let enc = ConstrPlutusDataEncoding {
        len_encoding: LenEncoding::Indefinite,
        ..ConstrPlutusDataEncoding::default()
    };
    PlutusData::new_constr_plutus_data(ConstrPlutusData {
        alternative: 0,
        fields,
        encodings: Some(enc),
    })
}

#[cfg(test)]
mod tests {
    use cml_chain::Serialize;
    use cml_crypto::Ed25519KeyHash;
    use cml_crypto::RawBytesEncoding;
    use cml_crypto::ScriptHash;
    use spectrum_cardano_lib::{AssetName, NetworkId};
    use spectrum_offchain_cardano::creds::operator_creds_base_address;
    use splash_dao_offchain::entities::{
        offchain::voting_order::{VotingOrder, VotingOrderId},
        onchain::voting_escrow::VotingEscrowId,
    };

    use super::create_voting_order;

    #[test]
    fn zzz() {
        let (addr, _, operator_pkh, _operator_cred, operator_sk) =
            operator_creds_base_address("xprv18zms3y4qkekv08jecrggtdvrxs3a2skf4cz7elfn8lsvq2nf39tqzd8mhh5kv9dp27kf3fy80uunz9k83rtg6gw5vvyu04f6tl8uw5pryslfc3g6wnjffneazpxh5t2nea7hq72hdsuc4m7ftry07yglwysze4ep", NetworkId::from(0));

        let voting_escrow_id = VotingEscrowId(AssetName::utf8_unsafe("ve_id".into()));
        let id = VotingOrderId {
            voting_escrow_id,
            version: 0,
        };

        let wpoll_policy_id =
            ScriptHash::from_hex("6da073591bfaffa99618d0b587434f5d7e681c4b78a70a53af816d98").unwrap();
        let VotingOrder {
            id,
            distribution,
            proof,
            witness,
            witness_input,
            version,
        } = create_voting_order(&operator_sk, id, 20, wpoll_policy_id, 2);

        let operator_pkh_str: String = operator_pkh.into();
        let pk_hash = operator_sk.to_public().to_raw_bytes().to_vec();
        println!("-----------payment_cred: {}", hex::encode(&pk_hash));
        println!("proof: {}", hex::encode(&proof));
        println!("witness: {}", witness.to_hex());
        //println!("witness_input: {}", hex::encode(witness_input.to_cbor_bytes()));
    }
}
