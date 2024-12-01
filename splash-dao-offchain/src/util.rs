use std::ops::Deref;

use cardano_explorer::CardanoNetwork;
use cml_chain::{
    address::Address,
    builders::{
        output_builder::TransactionOutputBuilder,
        tx_builder::{ChangeSelectionAlgo, SignedTxBuilder},
    },
    transaction::Transaction,
    Serialize, Value,
};
use cml_crypto::TransactionHash;
use log::trace;
use spectrum_cardano_lib::{
    collateral::Collateral, protocol_params::constant_tx_builder, transaction::OutboundTransaction, OutputRef,
};
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::prover::operator::OperatorProver;

use crate::{collateral::COLLATERAL_LOVELACES, collect_utxos::collect_utxos};

pub async fn generate_collateral<Net: CardanoNetwork, TX>(
    explorer: &Net,
    addr: &Address,
    collateral_addr: &Address,
    prover: &TX,
) -> Result<Collateral, Box<dyn std::error::Error>>
where
    TX: TxProver<SignedTxBuilder, OutboundTransaction<Transaction>>,
{
    let all_utxos = explorer.utxos_by_address(addr.clone(), 0, 100).await;
    let utxos = collect_utxos(all_utxos, COLLATERAL_LOVELACES + 1_000_000, vec![], None);

    let mut tx_builder = constant_tx_builder();
    for utxo in utxos {
        tx_builder.add_input(utxo).unwrap();
    }
    let output_result = TransactionOutputBuilder::new()
        .with_address(collateral_addr.clone())
        .next()
        .unwrap()
        .with_value(Value::from(COLLATERAL_LOVELACES))
        .build()
        .unwrap();
    tx_builder.add_output(output_result).unwrap();
    let signed_tx_builder = tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap();

    let tx = prover.prove(signed_tx_builder);
    let tx_hash = TransactionHash::from_hex(&tx.body.hash().to_hex()).unwrap();
    let tx_bytes = tx.to_cbor_bytes();
    trace!(
        "Generating collateral TX. TX hash: {:?}, TX bytes: {}",
        tx_hash,
        hex::encode(&tx_bytes)
    );

    explorer.submit_tx(&tx_bytes).await?;
    explorer.wait_for_transaction_confirmation(tx_hash).await?;

    let output_ref = OutputRef::new(tx_hash, 0);
    let utxo = explorer.utxo_by_ref(output_ref).await.unwrap();
    Ok(Collateral::from(utxo))
}
