use std::collections::VecDeque;

use cml_chain::{
    address::Address,
    assets::{AssetName, MultiAsset},
    builders::{
        input_builder::InputBuilderResult,
        mint_builder::SingleMintBuilder,
        output_builder::TransactionOutputBuilder,
        redeemer_builder::RedeemerWitnessKey,
        tx_builder::{ChangeSelectionAlgo, SignedTxBuilder},
        witness_builder::{NativeScriptWitnessInfo, PartialPlutusWitness},
    },
    min_ada::min_ada_required,
    plutus::{ConstrPlutusData, ExUnits, PlutusData, PlutusScript, PlutusV2Script, RedeemerTag},
    transaction::NativeScript,
    utils::BigInteger,
    Serialize, Value,
};
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding, TransactionHash};
use spectrum_cardano_lib::{
    collateral::Collateral,
    plutus_data::PlutusDataExtension,
    protocol_params::{constant_tx_builder, COINS_PER_UTXO_BYTE},
    transaction::TransactionOutputExtension,
    Token,
};
use spectrum_offchain::tx_hash;
use spectrum_offchain_cardano::{
    deployment::{RawCBORScript, Script, ScriptType},
    parametrized_validators::apply_params_validator,
};
use splash_dao_offchain::{
    constants::{DEFAULT_AUTH_TOKEN_NAME, GT_NAME, MAX_GT_SUPPLY, ONE_TIME_MINT_SCRIPT},
    deployment::{BuiltPolicy, MintedTokens},
};
use uplc_pallas_codec::minicbor::Encode;

use crate::ExternallyMintedToken;

pub fn mint_token(
    token_name: &str,
    quantity: i64,
    pk_hash: Ed25519KeyHash,
    input_utxo: InputBuilderResult,
    change_address: &Address,
    current_slot_number: u64,
) -> (SignedTxBuilder, ExternallyMintedToken) {
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
    tx_builder.add_input(input_utxo).unwrap();
    tx_builder.set_validity_start_interval(current_slot_number - 5);
    tx_builder.set_ttl(valid_until);

    let mut output_multiasset = MultiAsset::new();
    output_multiasset.set(script_all_hash, asset_name.clone(), quantity as u64);

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

    let signed_tx_builder = tx_builder
        .build(ChangeSelectionAlgo::Default, change_address)
        .unwrap();
    let token = Token(script_all_hash, spectrum_cardano_lib::AssetName::from(asset_name));
    let minted_token = ExternallyMintedToken {
        token,
        quantity: quantity as u64,
    };
    (signed_tx_builder, minted_token)
}

/// Create a TX to generate UTxO inputs for deployment tokens that need minting.
pub fn create_minting_tx_inputs(input_utxo: InputBuilderResult, addr: &Address) -> SignedTxBuilder {
    let mut tx_builder = constant_tx_builder();
    tx_builder.add_input(input_utxo).unwrap();
    let mut output_result = TransactionOutputBuilder::new()
        .with_address(addr.clone())
        .next()
        .unwrap()
        .with_value(Value::from(5_000_000))
        .build()
        .unwrap();
    let min_ada = min_ada_required(&output_result.output, COINS_PER_UTXO_BYTE).unwrap();
    let updated_value = Value::from(min_ada);
    output_result.output.update_value(updated_value);

    for _ in 0..NUMBER_TOKEN_MINTS_NEEDED {
        tx_builder.add_output(output_result.clone()).unwrap();
    }
    tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap()
}

pub fn mint_deployment_tokens(
    inputs: Vec<InputBuilderResult>,
    addr: &Address,
    public_key_hash: Ed25519KeyHash,
    collateral: Collateral,
) -> (SignedTxBuilder, MintedTokens) {
    let mut tx_builder = constant_tx_builder();
    for input in &inputs {
        tx_builder.add_input(input.clone()).unwrap();
    }

    let mut built_policies = VecDeque::new();

    // First 7 mints are for the NFTs, and the final mint is for GT.
    let qty = |index: usize| {
        if index < 7 {
            1
        } else {
            MAX_GT_SUPPLY
        }
    };

    let mut output_multiasset = MultiAsset::new();

    // Note that we have 7 NFTs to mint and the governance tokens too, hence the call to `.take(8)`.
    for (index, input_result) in inputs.iter().enumerate().take(8) {
        let tx_hash = input_result.input.transaction_id;
        let quantity = qty(index);
        let plutus_script = compute_one_time_mint_validator(tx_hash, index, quantity);
        let policy_id = plutus_script.hash();
        let script = Script {
            typ: ScriptType::PlutusV2,
            script: RawCBORScript::from(plutus_script.clone().inner),
        };
        let inner = if index < 7 {
            DEFAULT_AUTH_TOKEN_NAME.to_be_bytes().to_vec()
        } else {
            GT_NAME.to_be_bytes().to_vec()
        };
        let asset_name = AssetName::new(inner).unwrap();
        let bp = BuiltPolicy {
            script,
            policy_id,
            asset_name: asset_name.clone(),
            quantity: BigInteger::from(quantity),
        };
        built_policies.push_back(bp);

        let mint_redeemer = PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![]));

        let witness = PartialPlutusWitness::new(
            cml_chain::builders::witness_builder::PlutusScriptWitness::Script(PlutusScript::from(
                plutus_script,
            )),
            mint_redeemer,
        );
        let mint_builder_result = SingleMintBuilder::new_single_asset(asset_name.clone(), quantity as i64)
            .plutus_script(witness, vec![public_key_hash].into());
        tx_builder.add_mint(mint_builder_result).unwrap();
        output_multiasset.set(policy_id, asset_name.clone(), quantity);
        println!("index {}", index);
    }

    for index in 0..8 {
        tx_builder.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Mint, index as u64), EX_UNITS);
    }

    let mut output_result = TransactionOutputBuilder::new()
        .with_address(addr.clone())
        .next()
        .unwrap()
        .with_value(Value::new(5_000_000, output_multiasset))
        .build()
        .unwrap();
    let min_ada = min_ada_required(&output_result.output, COINS_PER_UTXO_BYTE).unwrap();
    let updated_value = Value::from(min_ada);
    output_result.output.update_value(updated_value);

    tx_builder.add_output(output_result).unwrap();

    tx_builder
        .add_collateral(InputBuilderResult::from(collateral))
        .unwrap();
    let signed_tx_builder = tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap();

    let minted_tokens = MintedTokens {
        factory_auth: built_policies.pop_front().unwrap(),
        wp_factory_auth: built_policies.pop_front().unwrap(),
        ve_factory_auth: built_policies.pop_front().unwrap(),
        perm_auth: built_policies.pop_front().unwrap(),
        proposal_auth: built_policies.pop_front().unwrap(),
        edao_msig: built_policies.pop_front().unwrap(),
        inflation_auth: built_policies.pop_front().unwrap(),
        gt: built_policies.pop_front().unwrap(),
    };
    (signed_tx_builder, minted_tokens)
}

fn compute_one_time_mint_validator(tx_hash: TransactionHash, index: usize, quantity: u64) -> PlutusV2Script {
    let tx_hash_constr_pd = uplc::PlutusData::Constr(uplc::Constr {
        tag: 121,
        any_constructor: None,
        fields: vec![uplc::PlutusData::BoundedBytes(
            uplc_pallas_codec::utils::PlutusBytes::from(tx_hash.to_raw_bytes().to_vec()),
        )],
    });

    let output_ref_pd = uplc::PlutusData::Constr(uplc::Constr {
        tag: 121,
        any_constructor: None,
        fields: vec![
            tx_hash_constr_pd,
            uplc::PlutusData::BigInt(uplc::BigInt::Int(uplc_pallas_codec::utils::Int::from(
                index as i64,
            ))),
        ],
    });

    let quantity_pd = uplc::PlutusData::BigInt(uplc::BigInt::Int(uplc_pallas_codec::utils::Int::from(
        quantity as i64,
    )));

    let params_pd = uplc::PlutusData::Array(vec![output_ref_pd, quantity_pd]);
    apply_params_validator(params_pd, ONE_TIME_MINT_SCRIPT)
    //let buf: Vec<u8> = vec![];
    //let mut encoder = uplc_pallas_codec::minicbor::Encoder::new(buf);
    //tx_hash_constr_pd.encode(&mut encoder, &mut ()).unwrap();
    //let pallas_bytes = encoder.writer();

    //// CML
    //let cml = PlutusData::new_constr_plutus_data(ConstrPlutusData::new(
    //    0,
    //    vec![PlutusData::new_integer(BigInteger::from(100_i64))],
    //))
    //.to_cbor_bytes();

    //println!("CML PD HEX: {}", hex::encode(&cml));

    //assert_eq!(cml, *pallas_bytes);
}

pub const LQ_NAME: &str = "SPLASH/ADA LQ*";
pub const SPLASH_NAME: &str = "SPLASH";
pub const NUMBER_TOKEN_MINTS_NEEDED: usize = 9;
const EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};

#[cfg(test)]
mod tests {
    use super::compute_one_time_mint_validator;

    #[test]
    fn ahhaha() {}
}
