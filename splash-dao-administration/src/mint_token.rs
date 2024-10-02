use cml_chain::{
    address::Address,
    assets::{AssetName, MultiAsset},
    builders::{
        input_builder::InputBuilderResult,
        mint_builder::SingleMintBuilder,
        output_builder::TransactionOutputBuilder,
        tx_builder::{ChangeSelectionAlgo, SignedTxBuilder},
        witness_builder::NativeScriptWitnessInfo,
    },
    min_ada::min_ada_required,
    transaction::NativeScript,
    Value,
};
use cml_crypto::Ed25519KeyHash;
use spectrum_cardano_lib::{
    protocol_params::{constant_tx_builder, COINS_PER_UTXO_BYTE},
    transaction::TransactionOutputExtension,
};

pub fn mint_token(
    token_name: &str,
    quantity: i64,
    pk_hash: Ed25519KeyHash,
    input_result: InputBuilderResult,
    change_address: &Address,
    current_slot_number: u64,
) -> SignedTxBuilder {
    let valid_until = current_slot_number + 10001;

    let script_pk = NativeScript::new_script_pubkey(pk_hash);
    let script_before = NativeScript::ScriptInvalidHereafter(
        cml_chain::transaction::ScriptInvalidHereafter::new(valid_until),
    );
    let script_all = NativeScript::new_script_all(vec![script_before, script_pk]);
    let script_all_hash = script_all.hash();

    let mut tx_builder = constant_tx_builder();
    let asset_name = AssetName::try_from(token_name.as_bytes().to_vec()).unwrap();
    let mint_token_result = SingleMintBuilder::new_single_asset(asset_name.clone(), quantity)
        .native_script(script_all, NativeScriptWitnessInfo::Vkeys(vec![pk_hash]));
    tx_builder.add_mint(mint_token_result).unwrap();
    tx_builder.add_input(input_result).unwrap();
    tx_builder.set_validity_start_interval(current_slot_number - 5);
    tx_builder.set_ttl(valid_until);

    let mut output_multiasset = MultiAsset::new();
    output_multiasset.set(script_all_hash, asset_name, quantity as u64);

    let mut output_result = TransactionOutputBuilder::new()
        .with_address(change_address.clone())
        .next()
        .unwrap()
        .with_value(Value::new(5_000_000, output_multiasset.clone()))
        .build()
        .unwrap();
    let min_ada = min_ada_required(&output_result.output, COINS_PER_UTXO_BYTE).unwrap();
    let updated_value = Value::new(min_ada, output_multiasset);
    output_result.output.update_value(updated_value);
    tx_builder.add_output(output_result).unwrap();

    tx_builder
        .build(ChangeSelectionAlgo::Default, change_address)
        .unwrap()
}

fn create_minting_tx_inputs() {}

pub const LQ_NAME: &str = "SPLASH/ADA LQ*";
pub const SPLASH_NAME: &str = "SPLASH";
