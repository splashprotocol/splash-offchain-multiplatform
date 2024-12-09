use std::ops::Deref;

use cardano_explorer::CardanoNetwork;
use cml_chain::{
    address::Address,
    builders::{
        output_builder::TransactionOutputBuilder,
        tx_builder::{ChangeSelectionAlgo, TransactionUnspentOutput},
    },
    Serialize, Value,
};
use cml_crypto::TransactionHash;
use spectrum_cardano_lib::{
    collateral::Collateral, protocol_params::constant_tx_builder, transaction::TransactionOutputExtension,
    value::ValueExtension, OutputRef,
};
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::{creds::CollateralAddress, prover::operator::OperatorProver};

use crate::{
    collect_utxos::collect_utxos,
    create_change_output::{ChangeOutputCreator, CreateChangeOutput},
    deployment::BuiltPolicy,
};

const LIMIT: u16 = 50;
pub const COLLATERAL_LOVELACES: u64 = 5_000_000;

/// For collateral we insist on a crisp 5 ADA in the UTxO.
pub async fn pull_collateral<Net: CardanoNetwork>(
    collateral_address: CollateralAddress,
    explorer: &Net,
) -> Option<Collateral> {
    let mut collateral: Option<TransactionUnspentOutput> = None;
    let mut offset = 0u32;
    let mut num_utxos_pulled = 0;
    while collateral.is_none() {
        let utxos = explorer
            .utxos_by_address(collateral_address.clone().address(), offset, LIMIT)
            .await;
        if utxos.is_empty() {
            break;
        }
        if utxos.len() > num_utxos_pulled {
            num_utxos_pulled = utxos.len();
        } else {
            // Didn't find any new UTxOs
            break;
        }
        if let Some(x) = utxos
            .into_iter()
            .find(|u| !u.output.amount().has_multiassets() && u.output.value().coin == COLLATERAL_LOVELACES)
        {
            collateral = Some(x);
        }
        offset += LIMIT as u32;
    }
    collateral.map(|out| out.into())
}

pub async fn send_assets<Net: CardanoNetwork>(
    coin_before_change_deduction: u64,
    change_output_coin: u64,
    required_tokens: Vec<BuiltPolicy>,
    explorer: &Net,
    wallet_addr: &Address,
    destination_addr: &Address,
    prover: &OperatorProver,
) -> Result<(), Box<dyn std::error::Error>> {
    let all_utxos = explorer.utxos_by_address(wallet_addr.clone(), 0, 100).await;
    let utxos = collect_utxos(
        all_utxos,
        coin_before_change_deduction,
        required_tokens.clone(),
        None,
    );
    let mut amount = 0;

    println!("wallet_addr: {}", wallet_addr.to_bech32(None).unwrap());
    let mut change_output_creator = ChangeOutputCreator::default();
    let mut tx_builder = constant_tx_builder();
    for (i, utxo) in utxos.into_iter().enumerate() {
        let utxo_coin = utxo.utxo_info.value().coin;
        amount += utxo_coin;
        println!("utxo #{}: {} lovelaces", i, utxo_coin);
        change_output_creator.add_input(&utxo);
        tx_builder.add_input(utxo).unwrap();
    }
    let mut output_value = Value::from(amount - change_output_coin);
    for BuiltPolicy {
        policy_id,
        asset_name,
        quantity,
    } in required_tokens
    {
        let asset_name = spectrum_cardano_lib::AssetName::from(asset_name);
        let ac = spectrum_cardano_lib::AssetClass::Token(spectrum_cardano_lib::Token(policy_id, asset_name));
        output_value.add_unsafe(ac, quantity.as_u64().unwrap());
    }
    let output_result = TransactionOutputBuilder::new()
        .with_address(destination_addr.clone())
        .next()
        .unwrap()
        .with_value(output_value)
        .build()
        .unwrap();
    change_output_creator.add_output(&output_result);
    tx_builder.add_output(output_result).unwrap();

    let estimated_tx_fee = tx_builder.min_fee(false).unwrap();
    let actual_fee = estimated_tx_fee + 200_000;
    let change_output = change_output_creator.create_change_output(actual_fee, wallet_addr.clone());
    tx_builder.set_fee(actual_fee);
    tx_builder.add_output(change_output).unwrap();

    let signed_tx_builder = tx_builder
        .build(ChangeSelectionAlgo::Default, wallet_addr)
        .unwrap();

    let tx = prover.prove(signed_tx_builder);
    let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.deref().to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await?;
    explorer.wait_for_transaction_confirmation(tx_hash).await?;

    Ok(())
}
