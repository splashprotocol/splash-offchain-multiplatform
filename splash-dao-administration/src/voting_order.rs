//! Code to create a voting_order for a user's voting_escrow
//!

use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::{ConstrPlutusData, PlutusData, PlutusScript, PlutusV2Script};
use cml_chain::utils::BigInteger;
use cml_chain::{LenEncoding, PolicyId, Serialize};
use cml_crypto::{PrivateKey, RawBytesEncoding, ScriptHash};
use splash_dao_offchain::constants::script_bytes::VOTING_WITNESS;
use splash_dao_offchain::entities::offchain::voting_order::compute_voting_witness_message;
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
    let redeemer = make_cml_witness_redeemer(&distribution, wpoll_policy_id, 0);
    let redeemer_pallas = make_pallas_redeemer(&distribution, wpoll_policy_id, 0);
    assert_eq!(
        hex::encode(redeemer.to_cbor_bytes()),
        hex::encode(redeemer_pallas.encode_fragment().unwrap()),
    );
    let message = compute_voting_witness_message(voting_witness_script.hash(), redeemer.clone(), 0);
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

fn make_cml_witness_redeemer(
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

    let distribution = distribution
        .iter()
        .map(|&(farm_id, weight)| {
            make_constr_pd_indefinite_arr(vec![
                PlutusData::new_bytes(cml_chain::assets::AssetName::from(farm_id.0).inner),
                PlutusData::new_integer(BigInteger::from(weight)),
            ])
        })
        .collect();

    let distribution_pd = PlutusData::List {
        list: distribution,
        list_encoding: LenEncoding::Indefinite,
    };
    make_constr_pd_indefinite_arr(vec![asset_pd, distribution_pd])
}

/// There are differences in how Aiken and default-CML serialise PlutusData. Here we'll form the
/// redeemer using the `PlutusData` representation from the Pallas crates, since it is used
/// internally by Aiken.
fn make_pallas_redeemer(
    distribution: &[(FarmId, u64)],
    wpoll_policy_id: ScriptHash,
    epoch: u32,
) -> uplc::PlutusData {
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

    let distribution = distribution
        .iter()
        .map(|&(farm_id, weight)| {
            to_plutus_cstr(vec![
                to_plutus_bytes(cml_chain::assets::AssetName::from(farm_id.0).inner),
                uplc::PlutusData::BigInt(uplc::BigInt::Int(uplc_pallas_codec::utils::Int::from(
                    weight as i64,
                ))),
            ])
        })
        .collect();

    let distribution_pd = uplc::PlutusData::Array(distribution);
    to_plutus_cstr(vec![asset_pd, distribution_pd])
}

fn make_constr_pd_indefinite_arr(fields: Vec<PlutusData>) -> PlutusData {
    let enc = ConstrPlutusDataEncoding {
        len_encoding: LenEncoding::Indefinite,
        prefer_compact: true,
        tag_encoding: None,
        alternative_encoding: None,
        fields_encoding: LenEncoding::Indefinite,
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
    use cml_crypto::ScriptHash;
    use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
    use splash_dao_offchain::routines::inflation::actions::compute_farm_name;
    use uplc_pallas_primitives::Fragment;

    use super::make_cml_witness_redeemer;
    use super::make_pallas_redeemer;

    #[test]
    fn test_cml_and_pallas_plutus_data_coincide() {
        let wpoll_policy_id =
            ScriptHash::from_hex("6da073591bfaffa99618d0b587434f5d7e681c4b78a70a53af816d98").unwrap();

        let distribution = generate_distribution(5);

        let epoch = 20;
        let cml_rdmr = make_cml_witness_redeemer(&distribution, wpoll_policy_id, epoch);
        let pallas_rdmr = make_pallas_redeemer(&distribution, wpoll_policy_id, epoch);
        assert_eq!(cml_rdmr.to_cbor_bytes(), pallas_rdmr.encode_fragment().unwrap());
    }

    fn generate_distribution(n: usize) -> Vec<(FarmId, u64)> {
        let mut dist = vec![];
        for i in 0..n {
            let farm_name = FarmId(spectrum_cardano_lib::AssetName::from(compute_farm_name(i as u32)));
            let p = ((i + 1) * 10) as u64;
            dist.push((farm_name, p));
        }

        dist
    }
}
