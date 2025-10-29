//! Code to create a voting_order for a user's voting_escrow
//!

use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::{ConstrPlutusData, PlutusData, PlutusScript, PlutusV3Script};
use cml_chain::utils::BigInteger;
use cml_chain::{LenEncoding, PolicyId, Serialize};
use cml_crypto::{PrivateKey, RawBytesEncoding, ScriptHash};
use rand::Rng;
use spectrum_cardano_lib::plutus_data::make_constr_pd_indefinite_arr;
use spectrum_cardano_lib::OutputRef;
use splash_dao_offchain::deployment::DaoScriptData;
use splash_dao_offchain::entities::offchain::{OffChainOrderId, WPollVoteOffChainOrder};
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::routines::actions::{compute_epoch_asset_name, compute_farm_name};
use uplc_pallas_primitives::{BoundedBytes, Fragment};

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

    let to_plutus_bytes = |bytes: Vec<u8>| uplc::PlutusData::BoundedBytes(BoundedBytes::from(bytes));
    let to_plutus_cstr = |fields: Vec<uplc::PlutusData>| {
        uplc::PlutusData::Constr(uplc::Constr {
            tag: 121,
            any_constructor: None,
            fields: uplc_pallas_primitives::MaybeIndefArray::Indef(fields),
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

    let distribution_pd =
        uplc::PlutusData::Array(uplc_pallas_primitives::MaybeIndefArray::Indef(distribution));
    to_plutus_cstr(vec![asset_pd, distribution_pd])
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
    use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
    use splash_dao_offchain::routines::actions::compute_farm_name;
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

    #[test]
    fn hex_test() {
        let bytes: Vec<u8> = vec![
            241, 100, 114, 32, 198, 101, 44, 85, 244, 110, 133, 129, 19, 109, 16, 231, 233, 86, 149, 17, 174,
            58, 5, 15, 215, 151, 222, 253, 21, 190, 117, 126,
        ];
        println!("{}", hex::encode(bytes));
    }
}
