use cml_chain::{
    address::EnterpriseAddress,
    assets::{AssetName, MultiAsset},
    byron::ProtocolMagic,
    certs::StakeCredential,
    genesis::network_info::NetworkInfo,
    NetworkId,
    OrderedHashMap,
    plutus::{ConstrPlutusData, PlutusData, PlutusV2Script},
    PolicyId, Script, transaction::DatumOption, utils::BigInt, Value,
};
use cml_crypto::{RawBytesEncoding, TransactionHash};
use cml_multi_era::babbage::{
    BabbageFormatTxOut, BabbageTransactionBody, BabbageTransactionOutput,
    cbor_encodings::BabbageTransactionBodyEncoding,
};
use log::info;
use rand::Rng;

use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::constants::POOL_V2_SCRIPT;
use spectrum_offchain_cardano::data::pool::CFMMPool;

use crate::{gen_policy_id, gen_transaction_input};

pub fn gen_ada_token_pool(lovelaces: u64, y_token_quantity: u64, ada_first: bool) -> CFMMPool {
    let (repr, _, _) = gen_pool_transaction_output(0, lovelaces, y_token_quantity, ada_first);
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes[..]);
    let transaction_id = TransactionHash::from(bytes);
    let ctx = OutputRef::new(transaction_id, 0);
    CFMMPool::try_from_ledger(&repr, ctx).unwrap()
}

pub fn gen_pool_transaction_body(
    network_id: NetworkId,
    lovelaces: u64,
    y_token_quantity: u64,
) -> (BabbageTransactionBody, PolicyId, &'static str) {
    let (output, token_policy, token_name) =
        gen_pool_transaction_output(0, lovelaces, y_token_quantity, false);
    let body = BabbageTransactionBody {
        inputs: vec![gen_transaction_input(0), gen_transaction_input(1)],
        outputs: vec![output],
        fee: 281564,
        ttl: None,
        certs: None,
        withdrawals: None,
        update: None,
        auxiliary_data_hash: None,
        validity_interval_start: Some(51438385),
        mint: None,
        script_data_hash: None,
        collateral_inputs: None,
        required_signers: None,
        network_id: Some(network_id),
        collateral_return: None,
        total_collateral: Some(3607615),
        reference_inputs: None,
        encodings: Some(BabbageTransactionBodyEncoding::default()),
    };
    (body, token_policy, token_name)
}

pub fn gen_pool_transaction_output(
    index: u64,
    lovelaces: u64,
    y_token_quantity: u64,
    ada_first: bool,
) -> (BabbageTransactionOutput, PolicyId, &'static str) {
    let network = NetworkInfo::new(0b0000, ProtocolMagic::from(1_u32)).network_id();
    let script_plutus_v2 = PlutusV2Script::new(hex::decode(POOL_V2_SCRIPT).unwrap());
    let script = Script::new_plutus_v2(script_plutus_v2.clone());
    let address =
        EnterpriseAddress::new(network, StakeCredential::new_script(script.clone().hash())).to_address();
    info!(
        "pool_v2_addr: {}, {:?}",
        address.to_bech32(None).unwrap(),
        address
    );

    let pool_y_token_policy = gen_policy_id();
    let pool_y_token_name = "token Y";
    let pool_lq_token_policy = gen_policy_id();
    let pool_lq_token_name = "token LQ";

    let mut bundle = OrderedHashMap::new();
    bundle.insert(
        pool_y_token_policy,
        OrderedHashMap::from_iter(vec![(
            AssetName::try_from(pool_y_token_name.as_bytes().to_vec()).unwrap(),
            y_token_quantity,
        )]),
    );
    bundle.insert(
        pool_lq_token_policy,
        OrderedHashMap::from_iter(vec![(
            AssetName::try_from(pool_lq_token_name.as_bytes().to_vec()).unwrap(),
            10_000_000,
        )]),
    );
    let amount = Value::new(lovelaces, MultiAsset::from(bundle));

    let datum_option = Some(DatumOption::new_datum(gen_pool_datum(
        pool_y_token_policy,
        pool_y_token_name,
        pool_lq_token_policy,
        pool_lq_token_name,
        ada_first,
    )));

    // Don't need to set script_reference since the code uses reference inputs for the pool
    let tx = BabbageFormatTxOut {
        address,
        amount,
        datum_option,
        script_reference: None,
        encodings: None,
    };
    (
        BabbageTransactionOutput::BabbageFormatTxOut(tx),
        pool_y_token_policy,
        pool_y_token_name,
    )
}

// Datum for an ADA-token pool
fn gen_pool_datum(
    pool_y_token_policy: PolicyId,
    pool_y_token_name: &str,
    pool_lq_token_policy: PolicyId,
    pool_lq_token_name: &str,
    ada_first: bool,
) -> PlutusData {
    // Pool NFT
    let nft_token_policy = PlutusData::new_bytes(gen_policy_id().to_raw_bytes().to_vec());
    let nft_token_name = PlutusData::new_bytes(b"pool_NFT".to_vec());
    let nft_output =
        PlutusData::new_constr_plutus_data(ConstrPlutusData::new(0, vec![nft_token_policy, nft_token_name]));

    // ADA (pool X)
    let ada_bytes = PlutusData::new_bytes(vec![]);
    let ada_name_bytes = PlutusData::new_bytes(vec![]);

    let pool_x_output =
        PlutusData::new_constr_plutus_data(ConstrPlutusData::new(0, vec![ada_bytes, ada_name_bytes]));

    // Pool Y
    let pool_y_token_policy = PlutusData::new_bytes(pool_y_token_policy.to_raw_bytes().to_vec());
    let pool_y_token_name = PlutusData::new_bytes(pool_y_token_name.as_bytes().to_vec());
    let pool_y_output = PlutusData::new_constr_plutus_data(ConstrPlutusData::new(
        0,
        vec![pool_y_token_policy, pool_y_token_name],
    ));

    // Pool LQ
    let pool_lq_token_policy = PlutusData::new_bytes(pool_lq_token_policy.to_raw_bytes().to_vec());
    let pool_lq_token_name = PlutusData::new_bytes(pool_lq_token_name.as_bytes().to_vec());
    let pool_lq_output = PlutusData::new_constr_plutus_data(ConstrPlutusData::new(
        0,
        vec![pool_lq_token_policy, pool_lq_token_name],
    ));

    // Fee
    let fee_output = PlutusData::new_integer(BigInt::from(99950));

    // Dummy value
    let dummy_output = PlutusData::new_integer(BigInt::from(1));

    // LQ lower bound
    let lq_lower_bound_output = PlutusData::new_integer(BigInt::from(10000000000_u64));

    let fields = if ada_first {
        vec![
            nft_output,
            pool_x_output,
            pool_y_output,
            pool_lq_output,
            fee_output,
            dummy_output,
            lq_lower_bound_output,
        ]
    } else {
        vec![
            nft_output,
            pool_y_output,
            pool_x_output,
            pool_lq_output,
            fee_output,
            dummy_output,
            lq_lower_bound_output,
        ]
    };

    PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, fields))
}
