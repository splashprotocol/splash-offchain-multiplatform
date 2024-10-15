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
    OutputRef,
};
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::{creds::CollateralAddress, prover::operator::OperatorProver};

use crate::collect_utxos::collect_utxos;

const LIMIT: u16 = 50;
const COLLATERAL_LOVELACES: u64 = 5_000_000;

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

pub async fn generate_collateral<Net: CardanoNetwork>(
    explorer: &Net,
    addr: &Address,
    prover: &OperatorProver,
) -> Result<Collateral, Box<dyn std::error::Error>> {
    let all_utxos = explorer.utxos_by_address(addr.clone(), 0, 50).await;
    let utxos = collect_utxos(all_utxos, COLLATERAL_LOVELACES + 1_000_000, vec![], None);

    let mut tx_builder = constant_tx_builder();
    for utxo in utxos {
        tx_builder.add_input(utxo).unwrap();
    }
    let output_result = TransactionOutputBuilder::new()
        .with_address(addr.clone())
        .next()
        .unwrap()
        .with_value(Value::from(COLLATERAL_LOVELACES))
        .build()
        .unwrap();
    tx_builder.add_output(output_result).unwrap();
    let signed_tx_builder = tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap();

    let tx = prover.prove(signed_tx_builder);
    let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
    println!("Generating collateral TX ----------------------------------------------");
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.deref().to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await?;
    explorer.wait_for_transaction_confirmation(tx_hash).await?;

    let output_ref = OutputRef::new(tx_hash, 0);
    let utxo = explorer.utxo_by_ref(output_ref).await.unwrap();
    Ok(Collateral::from(utxo))
}
