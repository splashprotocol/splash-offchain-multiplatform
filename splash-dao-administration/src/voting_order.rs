//! Code to create a voting_order for a user's voting_escrow
//!

use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::{ConstrPlutusData, PlutusData, PlutusScript, PlutusV2Script};
use cml_chain::utils::BigInteger;
use cml_chain::{LenEncoding, PolicyId, Serialize};
use cml_crypto::{PrivateKey, RawBytesEncoding, ScriptHash};
use rand::Rng;
use spectrum_cardano_lib::plutus_data::make_constr_pd_indefinite_arr;
use splash_dao_offchain::deployment::DaoScriptData;
use splash_dao_offchain::entities::offchain::compute_voting_escrow_witness_message;
use splash_dao_offchain::entities::{
    offchain::voting_order::{VotingOrder, VotingOrderId},
    onchain::smart_farm::FarmId,
};
use splash_dao_offchain::routines::inflation::actions::{compute_epoch_asset_name, compute_farm_name};
use uplc_pallas_primitives::Fragment;

pub fn create_voting_order(
    operator_sk: &PrivateKey,
    id: VotingOrderId,
    voting_power: u64,
    wpoll_policy_id: PolicyId,
    epoch: u32,
    num_farms: u32,
) -> VotingOrder {
    let voting_witness_script = PlutusScript::PlutusV2(PlutusV2Script::new(
        hex::decode(&DaoScriptData::global().voting_witness.script_bytes).unwrap(),
    ));

    // Randomly choose a farm to apply the full weight towards
    let mut rng = rand::thread_rng();
    let chosen_id = rng.gen_range(0..num_farms);
    let distribution: Vec<_> = (0..num_farms)
        .filter_map(|id| {
            if id == chosen_id {
                Some((
                    FarmId(spectrum_cardano_lib::AssetName::from(compute_farm_name(id))),
                    voting_power,
                ))
            } else {
                None
            }
        })
        .collect();

    let redeemer = make_cml_witness_redeemer(&distribution, wpoll_policy_id, epoch);
    let redeemer_pallas = make_pallas_redeemer(&distribution, wpoll_policy_id, epoch);

    assert_eq!(
        hex::encode(redeemer.to_cbor_bytes()),
        hex::encode(redeemer_pallas.encode_fragment().unwrap()),
    );
    println!("witness_script hash: {}", voting_witness_script.hash().to_hex());
    let redeemer_hex = hex::encode(redeemer.to_cbor_bytes());
    println!("redeemer: {}", redeemer_hex);
    let message =
        compute_voting_escrow_witness_message(voting_witness_script.hash(), redeemer_hex.clone(), id.version)
            .unwrap();
    println!("message: {}", hex::encode(&message));
    let signature = operator_sk.sign(&message).to_raw_bytes().to_vec();

    VotingOrder {
        id,
        distribution,
        proof: signature,
        witness: voting_witness_script.hash(),
        witness_input: redeemer_hex,
        version: id.version as u32,
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

#[cfg(test)]
mod tests {
    use cml_chain::plutus::PlutusData;
    use cml_chain::Deserialize;
    use cml_chain::PolicyId;
    use cml_chain::Serialize;
    use cml_crypto::RawBytesEncoding;
    use cml_crypto::ScriptHash;
    use spectrum_cardano_lib::NetworkId;
    use spectrum_offchain_cardano::creds::operator_creds_base_address;
    use splash_dao_offchain::entities::offchain::compute_voting_escrow_witness_message;
    use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
    use splash_dao_offchain::routines::inflation::actions::compute_farm_name;
    use uplc_pallas_primitives::Fragment;

    use crate::mint_token::script_address;
    use crate::OperatorProver;

    use super::make_cml_witness_redeemer;
    use super::make_pallas_redeemer;

    #[test]
    fn test_cml_and_pallas_plutus_data_coincide() {
        let wpoll_policy_id =
            ScriptHash::from_hex("6da073591bfaffa99618d0b587434f5d7e681c4b78a70a53af816d98").unwrap();

        let pk = cml_crypto::PublicKey::from_raw_hex(
            "f27b7e514487a1f862c48889127289add9d96ce246c70c411df7fb46c4ec1225",
        )
        .unwrap();
        let mut distribution = generate_distribution(1);
        distribution[0].1 = 20;

        let epoch = 0;
        let cml_rdmr = make_cml_witness_redeemer(&distribution, wpoll_policy_id, epoch);
        let pallas_rdmr = make_pallas_redeemer(&distribution, wpoll_policy_id, epoch);
        assert_eq!(cml_rdmr.to_cbor_bytes(), pallas_rdmr.encode_fragment().unwrap());

        let redeemer_hex = hex::encode(cml_rdmr.to_cbor_bytes());
        println!("redeemer: {}", redeemer_hex);

        let cml_cbor_bytes = cml_rdmr.to_cbor_bytes();
        assert_eq!(cml_rdmr, PlutusData::from_cbor_bytes(&cml_cbor_bytes).unwrap());

        let witness_sh =
            ScriptHash::from_hex("9e7637b80d1df227ec2061a88e7720df831c9fe9a2163a0334099d9e").unwrap();
        let message = compute_voting_escrow_witness_message(witness_sh, redeemer_hex.clone(), 0).unwrap();
        println!("message: {}", hex::encode(&message));

        let (addr, _, operator_pkh, _operator_cred, operator_sk) =
        operator_creds_base_address("xprv18zms3y4qkekv08jecrggtdvrxs3a2skf4cz7elfn8lsvq2nf39tqzd8mhh5kv9dp27kf3fy80uunz9k83rtg6gw5vvyu04f6tl8uw5pryslfc3g6wnjffneazpxh5t2nea7hq72hdsuc4m7ftry07yglwysze4ep",
    NetworkId::from(0) );
        let sk_bech32 = operator_sk.to_bech32();
        let prover = OperatorProver::new(sk_bech32);
        //let prover = OperatorProver::new(config.batcher_private_key.into());
        let owner_pub_key = operator_sk.to_public();
        assert_eq!(pk, owner_pub_key);

        let signature = operator_sk.sign(&message).to_raw_bytes().to_vec();
        println!("sig: {}", hex::encode(signature));
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
