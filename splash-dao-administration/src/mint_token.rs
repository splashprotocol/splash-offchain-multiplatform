use std::collections::VecDeque;

use cml_chain::{
    address::{Address, BaseAddress, EnterpriseAddress},
    assets::{AssetName, MultiAsset},
    builders::{
        input_builder::InputBuilderResult,
        mint_builder::SingleMintBuilder,
        output_builder::TransactionOutputBuilder,
        redeemer_builder::RedeemerWitnessKey,
        tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionBuilder},
        witness_builder::{NativeScriptWitnessInfo, PartialPlutusWitness},
    },
    certs::StakeCredential,
    min_ada::min_ada_required,
    plutus::{ConstrPlutusData, ExUnits, PlutusData, PlutusScript, PlutusV2Script, RedeemerTag},
    transaction::NativeScript,
    utils::BigInteger,
    PolicyId, Serialize, Value,
};
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding, ScriptHash, TransactionHash};
use spectrum_cardano_lib::{
    collateral::Collateral,
    plutus_data::PlutusDataExtension,
    protocol_params::{constant_tx_builder, COINS_PER_UTXO_BYTE},
    transaction::TransactionOutputExtension,
    NetworkId, Token,
};
use spectrum_offchain::tx_hash;
use spectrum_offchain_cardano::{
    deployment::{RawCBORScript, Script, ScriptType},
    parametrized_validators::apply_params_validator,
};
use splash_dao_offchain::{
    constants::{
        DEFAULT_AUTH_TOKEN_NAME, GOV_PROXY_SCRIPT, GT_NAME, MAX_GT_SUPPLY, MINT_IDENTIFIER_SCRIPT,
        MINT_VE_COMPOSITION_TOKEN_SCRIPT, ONE_TIME_MINT_SCRIPT,
    },
    deployment::{BuiltPolicy, MintedTokens},
    entities::onchain::{
        farm_factory::compute_farm_factory_validator,
        inflation_box::compute_inflation_box_validator,
        permission_manager::compute_perm_manager_validator,
        poll_factory::{compute_wp_factory_validator, PollFactoryConfig},
        smart_farm::compute_mint_farm_auth_token_validator,
        voting_escrow::{
            compute_mint_governance_power_validator, compute_mint_weighting_power_validator,
            compute_voting_escrow_validator, VotingEscrow, VotingEscrowConfig,
        },
        voting_escrow_factory::compute_ve_factory_validator,
        weighting_poll::compute_mint_wp_auth_token_validator,
    },
};
use uplc_pallas_codec::{minicbor::Encode, utils::PlutusBytes};

use crate::{ExternallyMintedToken, PreprodDeploymentProgress};

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

    // Even though we've specified it, the tokens do not appear in this output.
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

/// Computes the scripts of all DAO reference inputs, and forms `TransactionBuilder` instances containing
/// the necessary outputs for reference input UTxOs.
pub fn create_dao_reference_input_utxos(
    config: &PreprodDeploymentProgress,
    zeroth_epoch_start: u64,
) -> (
    TransactionBuilder,
    TransactionBuilder,
    TransactionBuilder,
    ReferenceInputScriptHashes,
) {
    let minted_tokens = config.minted_deployment_tokens.as_ref().unwrap();
    let gt_policy = minted_tokens.gt.policy_id;
    let ve_factory_auth_policy = minted_tokens.ve_factory_auth.policy_id;
    let proposal_auth_policy = minted_tokens.proposal_auth.policy_id;
    let perm_manager_auth_policy = minted_tokens.perm_auth.policy_id;
    let edao_msig = minted_tokens.edao_msig.policy_id;
    let inflation_auth_policy = minted_tokens.inflation_auth.policy_id;

    let governance_power_script = compute_mint_governance_power_validator(proposal_auth_policy, gt_policy);

    let gov_proxy_script = compute_gov_proxy_script(
        ve_factory_auth_policy,
        proposal_auth_policy,
        governance_power_script.hash(),
        gt_policy,
    );

    let mint_ve_composition_token_script = compute_mint_ve_composition_token_script(ve_factory_auth_policy);

    let voting_escrow_script =
        compute_voting_escrow_validator(ve_factory_auth_policy, mint_ve_composition_token_script.hash());

    let farm_factory_auth_policy = minted_tokens.factory_auth.policy_id;

    let splash_policy = config.splash_tokens.as_ref().unwrap().token.0;

    let mint_farm_auth_token_script =
        compute_mint_farm_auth_token_validator(splash_policy, farm_factory_auth_policy);

    let wp_factory_auth_policy = minted_tokens.wp_factory_auth.policy_id;
    let mint_wp_auth_token_script = compute_mint_wp_auth_token_validator(
        splash_policy,
        mint_farm_auth_token_script.hash(),
        wp_factory_auth_policy,
        inflation_auth_policy,
        zeroth_epoch_start,
    );

    let mint_identifier_script = PlutusV2Script::new(hex::decode(MINT_IDENTIFIER_SCRIPT).unwrap());

    let ve_factory_script = compute_ve_factory_validator(
        ve_factory_auth_policy,
        mint_identifier_script.hash(),
        mint_ve_composition_token_script.hash(),
        gt_policy,
        voting_escrow_script.hash(),
        gov_proxy_script.hash(),
    );

    let wp_factory_script =
        compute_wp_factory_validator(mint_wp_auth_token_script.hash(), gov_proxy_script.hash());

    let farm_factory_script =
        compute_farm_factory_validator(mint_farm_auth_token_script.hash(), gov_proxy_script.hash());

    let mint_weighting_power_script =
        compute_mint_weighting_power_validator(zeroth_epoch_start, proposal_auth_policy, gt_policy);

    let inflation_script = compute_inflation_box_validator(
        splash_policy,
        mint_wp_auth_token_script.hash(),
        mint_weighting_power_script.hash(),
        zeroth_epoch_start,
    );

    let perm_manager_script = compute_perm_manager_validator(edao_msig, perm_manager_auth_policy);

    let reference_input_script_hashes = ReferenceInputScriptHashes {
        inflation: inflation_script.hash(),
        voting_escrow: voting_escrow_script.hash(),
        farm_factory: farm_factory_script.hash(),
        wp_factory: wp_factory_script.hash(),
        ve_factory: ve_factory_script.hash(),
        gov_proxy: gov_proxy_script.hash(),
        perm_manager: perm_manager_script.hash(),
        mint_wpauth_token: mint_wp_auth_token_script.hash(),
        mint_identifier: mint_identifier_script.hash(),
        mint_ve_composition_token: mint_ve_composition_token_script.hash(),
        weighting_power: mint_weighting_power_script.hash(),
        smart_farm: mint_farm_auth_token_script.hash(),
    };

    let script_before =
        NativeScript::ScriptInvalidHereafter(cml_chain::transaction::ScriptInvalidHereafter::new(0));

    let script_addr = script_address(script_before.hash(), config.network_id);

    let make_output = |script| {
        TransactionOutputBuilder::new()
            .with_address(script_addr.clone())
            .with_reference_script(cml_chain::Script::new_plutus_v2(script))
            .next()
            .unwrap()
            .with_asset_and_min_required_coin(MultiAsset::default(), COINS_PER_UTXO_BYTE)
            .unwrap()
            .build()
            .unwrap()
    };

    let mut tx_builder_0 = constant_tx_builder();
    tx_builder_0.add_output(make_output(inflation_script)).unwrap();
    tx_builder_0
        .add_output(make_output(voting_escrow_script))
        .unwrap();
    tx_builder_0.add_output(make_output(farm_factory_script)).unwrap();
    tx_builder_0.add_output(make_output(wp_factory_script)).unwrap();
    tx_builder_0.add_output(make_output(ve_factory_script)).unwrap();

    let mut tx_builder_1 = constant_tx_builder();
    tx_builder_1.add_output(make_output(gov_proxy_script)).unwrap();
    tx_builder_1.add_output(make_output(perm_manager_script)).unwrap();
    tx_builder_1
        .add_output(make_output(mint_wp_auth_token_script))
        .unwrap();
    tx_builder_1
        .add_output(make_output(mint_identifier_script))
        .unwrap();
    let mut tx_builder_2 = constant_tx_builder();
    tx_builder_2
        .add_output(make_output(mint_ve_composition_token_script))
        .unwrap();
    tx_builder_2
        .add_output(make_output(mint_weighting_power_script))
        .unwrap();
    tx_builder_2
        .add_output(make_output(mint_farm_auth_token_script))
        .unwrap();

    (
        tx_builder_0,
        tx_builder_1,
        tx_builder_2,
        reference_input_script_hashes,
    )
}

fn compute_gov_proxy_script(
    ve_factory_auth_policy: PolicyId,
    proposal_auth_policy: PolicyId,
    governance_power_policy: PolicyId,
    gt_policy: PolicyId,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(ve_factory_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(proposal_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(governance_power_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(gt_policy.to_raw_bytes().to_vec())),
    ]);
    apply_params_validator(params_pd, GOV_PROXY_SCRIPT)
}

fn compute_mint_ve_composition_token_script(ve_factory_auth_policy: PolicyId) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(vec![uplc::PlutusData::BoundedBytes(PlutusBytes::from(
        ve_factory_auth_policy.to_raw_bytes().to_vec(),
    ))]);
    apply_params_validator(params_pd, MINT_VE_COMPOSITION_TOKEN_SCRIPT)
}

pub fn script_address(script_hash: ScriptHash, network_id: NetworkId) -> Address {
    EnterpriseAddress::new(u8::from(network_id), StakeCredential::new_script(script_hash)).to_address()
}

pub const LQ_NAME: &str = "SPLASH/ADA LQ*";
pub const SPLASH_NAME: &str = "SPLASH";
pub const NUMBER_TOKEN_MINTS_NEEDED: usize = 9;
const EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};

pub struct ReferenceInputScriptHashes {
    pub inflation: ScriptHash,
    pub voting_escrow: ScriptHash,
    pub farm_factory: ScriptHash,
    pub wp_factory: ScriptHash,
    pub ve_factory: ScriptHash,
    pub gov_proxy: ScriptHash,
    pub perm_manager: ScriptHash,
    pub mint_wpauth_token: ScriptHash,
    pub mint_identifier: ScriptHash,
    pub mint_ve_composition_token: ScriptHash,
    pub weighting_power: ScriptHash,
    pub smart_farm: ScriptHash,
}

pub struct DaoDeploymentParameters {
    zeroth_epoch_start: u64,
    wp_factory_config: PollFactoryConfig,
    voting_escrow: VotingEscrowConfig,
}

#[cfg(test)]
mod tests {
    use super::compute_one_time_mint_validator;

    #[test]
    fn ahhaha() {}
}
