use cardano_chain_sync::data::LedgerTxEvent;
use cml_chain::{
    address::EnterpriseAddress,
    assets::{AssetName, MultiAsset},
    byron::ProtocolMagic,
    certs::StakeCredential,
    genesis::network_info::NetworkInfo,
    plutus::{ConstrPlutusData, PlutusData, PlutusV2Script},
    transaction::DatumOption,
    utils::BigInt,
    NetworkId, OrderedHashMap, PolicyId, Script, Value,
};
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
use cml_multi_era::babbage::{
    cbor_encodings::BabbageTransactionBodyEncoding, BabbageFormatTxOut, BabbageTransaction,
    BabbageTransactionBody, BabbageTransactionOutput, BabbageTransactionWitnessSet,
};
use log::info;
use rand::{thread_rng, Rng};
use spectrum_offchain_cardano::{constants::SPOT_SCRIPT, creds::operator_creds};

use crate::{gen_policy_id, gen_transaction_input};

pub fn gen_spot_tx(
    network_id: NetworkId,
    output_token_policy: PolicyId,
    output_token_name: &str,
    slot: u64,
    num_orders: usize,
    min_tradable_lovelaces: u64,
    max_tradable_lovelaces: u64,
) -> LedgerTxEvent<BabbageTransaction> {
    let tx = gen_spot_order_babbage_tx(
        network_id,
        output_token_policy,
        output_token_name,
        min_tradable_lovelaces,
        max_tradable_lovelaces,
        num_orders,
    );
    LedgerTxEvent::TxApplied { tx, slot }
}

fn gen_spot_order_babbage_tx(
    network_id: NetworkId,
    output_token_policy: PolicyId,
    output_token_name: &str,
    min_tradable_lovelaces: u64,
    max_tradable_lovelaces: u64,
    num_orders: usize,
) -> BabbageTransaction {
    let witness_set = BabbageTransactionWitnessSet::new();
    let mut outputs = vec![];
    for _ in 0..num_orders {
        let tradable_input = thread_rng().gen_range(min_tradable_lovelaces..=max_tradable_lovelaces);
        outputs.push(gen_spot_order_transaction_output(
            output_token_policy,
            output_token_name,
            tradable_input,
        ));
    }
    let body = BabbageTransactionBody {
        inputs: vec![gen_transaction_input(0), gen_transaction_input(1)],
        outputs,
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
    BabbageTransaction::new(body, witness_set, true, None)
}

fn gen_spot_order_transaction_output(
    output_token_policy: PolicyId,
    output_token_name: &str,
    tradable_input: u64,
) -> BabbageTransactionOutput {
    let network_info = NetworkInfo::new(0b0000, ProtocolMagic::from(1_u32));
    let network = network_info.network_id();
    let (operator_sk, operator_pkh, operator_addr) = operator_creds("xprv1cr38ar6z5tn4mcjfa2pq49s2cchjpu6nd9qsqu6zxrruqu8kzfduch8repn8ukxdl4qjj4n002rwgf6dhg4ldq23vgsevt6tmnyc657yrsk5t6v3slm33qkh3f0x4xru6ue8w0k0medspw7fqfrmrppm0sdla648", 1_u64);
    let spot_script_inner = PlutusV2Script::new(hex::decode(SPOT_SCRIPT).unwrap());

    let script = Script::new_plutus_v2(spot_script_inner.clone());
    let address =
        EnterpriseAddress::new(network, StakeCredential::new_script(script.clone().hash())).to_address();
    info!(target:"offchain", "spot order address: {:?}", address);

    let beacon_token_policy = gen_policy_id();
    let beacon_token_name = "beacon";

    let mut bundle = OrderedHashMap::new();
    bundle.insert(
        beacon_token_policy,
        OrderedHashMap::from_iter(vec![(
            AssetName::try_from(beacon_token_name.as_bytes().to_vec()).unwrap(),
            1,
        )]),
    );
    bundle.insert(
        output_token_policy,
        OrderedHashMap::from_iter(vec![(
            AssetName::try_from(output_token_name.as_bytes().to_vec()).unwrap(),
            1,
        )]),
    );
    let amount = Value::new(tradable_input + 2_123_130, MultiAsset::from(bundle));

    let datum_option = Some(DatumOption::new_datum(gen_spot_order_datum(
        &beacon_token_policy,
        &operator_pkh,
        output_token_policy,
        output_token_name,
        tradable_input,
    )));

    let tx = BabbageFormatTxOut {
        address,
        amount,
        datum_option,
        script_reference: None,
        encodings: None,
    };
    BabbageTransactionOutput::BabbageFormatTxOut(tx)
}

fn gen_spot_order_datum(
    beacon: &PolicyId,
    redeemer_pkh: &Ed25519KeyHash,
    output_token_policy: PolicyId,
    output_token_name: &str,
    tradable_input: u64,
) -> PlutusData {
    let beacon_pd = PlutusData::new_bytes(beacon.to_raw_bytes().to_vec());
    let fee = 100_000;
    let cost_per_ex_step = 300_000;
    let tradable_input_pd = PlutusData::new_integer(BigInt::from(tradable_input));
    let cost_per_ex_step_pd = PlutusData::new_integer(BigInt::from(cost_per_ex_step));
    let max_ex_steps = 5;
    let base_price_num = 100;
    let base_price_denom = 100;
    let min_final_output = tradable_input * base_price_num / base_price_denom;
    let min_marginal_output = min_final_output / max_ex_steps;
    let min_marginal_output_pd = PlutusData::new_integer(BigInt::from(min_marginal_output));

    let output_token_policy = PlutusData::new_bytes(output_token_policy.to_raw_bytes().to_vec());
    let output_token_name = PlutusData::new_bytes(output_token_name.as_bytes().to_vec());
    let output = PlutusData::new_constr_plutus_data(ConstrPlutusData::new(
        0,
        vec![output_token_policy, output_token_name],
    ));

    let num_pd = PlutusData::new_integer(BigInt::from(base_price_num));
    let denom_pd = PlutusData::new_integer(BigInt::from(base_price_denom));
    let base_price_pd = PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![num_pd, denom_pd]));

    let fee_per_output = PlutusData::new_integer(BigInt::from(fee));
    let redeemer_reward_pkh = PlutusData::new_bytes(redeemer_pkh.to_raw_bytes().to_vec());
    let permitted_executors = PlutusData::new_list(vec![]);
    PlutusData::ConstrPlutusData(ConstrPlutusData::new(
        0,
        vec![
            beacon_pd,
            tradable_input_pd,
            cost_per_ex_step_pd,
            min_marginal_output_pd,
            output,
            base_price_pd,
            fee_per_output,
            redeemer_reward_pkh,
            permitted_executors,
        ],
    ))
}
