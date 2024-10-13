pub mod create_change_output;
mod mint_token;

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    hash::Hash,
    ops::Deref,
};

use cardano_explorer::{CardanoNetwork, Maestro};
use clap::{command, Parser};
use cml_chain::{
    address::Address,
    assets::MultiAsset,
    builders::{
        input_builder::{InputBuilderResult, SingleInputBuilder},
        mint_builder::SingleMintBuilder,
        output_builder::TransactionOutputBuilder,
        redeemer_builder::RedeemerWitnessKey,
        tx_builder::{ChangeSelectionAlgo, TransactionBuilder, TransactionUnspentOutput},
        witness_builder::{PartialPlutusWitness, PlutusScriptWitness},
    },
    plutus::{ConstrPlutusData, PlutusData, RedeemerTag},
    transaction::{DatumOption, TransactionOutput},
    utils::BigInteger,
    Coin, Serialize, Value,
};
use cml_crypto::{blake2b256, Ed25519KeyHash, RawBytesEncoding, ScriptHash, TransactionHash};
use create_change_output::{ChangeOutputCreator, CreateChangeOutput};
use mint_token::{script_address, DaoDeploymentParameters, LQ_NAME};
use spectrum_cardano_lib::{
    collateral::Collateral,
    ex_units::ExUnits,
    protocol_params::{constant_tx_builder, COINS_PER_UTXO_BYTE},
    transaction::TransactionOutputExtension,
    value::ValueExtension,
    AssetClass, NetworkId, OutputRef, Token,
};
use spectrum_cardano_lib::{plutus_data::IntoPlutusData, types::TryFromPData};
use spectrum_offchain::{data::Has, ledger::TryFromLedger, tx_prover::TxProver};
use spectrum_offchain_cardano::{
    creds::{operator_creds_base_address, CollateralAddress},
    deployment::{DeployedScriptInfo, DeployedValidatorRef, ReferenceUTxO},
    prover::operator::OperatorProver,
};
use splash_dao_offchain::{
    constants::{DEFAULT_AUTH_TOKEN_NAME, SPLASH_NAME},
    deployment::{BuiltPolicy, DeployedValidators, MintedTokens, ProtocolDeployment, ProtocolValidator},
    entities::onchain::{
        farm_factory::{FarmFactoryAction, FarmFactoryDatum},
        inflation_box::InflationBoxSnapshot,
        permission_manager::PermManagerSnapshot,
        poll_factory::{PollFactoryConfig, PollFactorySnapshot},
        smart_farm::MintAction,
        voting_escrow::{Lock, Owner, VotingEscrowConfig},
        voting_escrow_factory::{self, exchange_outputs, VEFactoryDatum, VEFactorySnapshot},
    },
    protocol_config::{GTAuthPolicy, VEFactoryAuthPolicy},
    time::NetworkTimeProvider,
    NetworkTimeSource,
};
use tokio::io::AsyncWriteExt;
use type_equalities::IsEqual;
use uplc_pallas_traverse::output;

const INFLATION_BOX_INITIAL_SPLASH_QTY: i64 = 32000000000000;

#[tokio::main]
async fn main() {
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    deploy(config).await;
}

async fn deploy<'a>(config: AppConfig<'a>) {
    let explorer = Maestro::new(config.maestro_key_path, config.network_id.into())
        .await
        .expect("Maestro instantiation failed");

    let (addr, _, operator_pkh, operator_cred, operator_sk) =
        operator_creds_base_address(config.batcher_private_key, config.network_id);

    let pk_bech32 = operator_sk.to_bech32();
    println!("pk_bech32: {}", pk_bech32);
    let prover = OperatorProver::new(pk_bech32);

    let collateral = if let Some(c) = pull_collateral(addr.clone().into(), &explorer).await {
        c
    } else {
        generate_collateral(&explorer, &addr, &prover).await.unwrap()
    };

    println!("Collateral output_ref: {}", collateral.reference());

    let operator_pkh_str: String = operator_pkh.into();

    let pk_hash = Ed25519KeyHash::from_bech32(&operator_pkh_str).unwrap();

    let input_result = get_largest_utxo(&explorer, &addr).await;

    let dao_parameters_str =
        std::fs::read_to_string(config.parameters_json_path).expect("Cannot load dao parameters file");
    let dao_parameters: DaoDeploymentParameters =
        serde_json::from_str(&dao_parameters_str).expect("Invalid parameters file");

    let raw_deployment_config =
        std::fs::read_to_string(config.deployment_json_path).expect("Cannot load configuration file");
    let mut deployment_config: PreprodDeploymentProgress =
        serde_json::from_str(&raw_deployment_config).expect("Invalid configuration file");

    // Mint LQ token -------------------------------------------------------------------------------
    if deployment_config.lq_tokens.is_none() {
        println!("Minting LQ tokens ----------------------------------------------------------");
        let (signed_tx_builder, minted_token) = mint_token::mint_token(
            LQ_NAME,
            INFLATION_BOX_INITIAL_SPLASH_QTY,
            pk_hash,
            input_result,
            &addr,
            explorer.chain_tip_slot_number().await.unwrap(),
        );
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
        println!("tx_hash: {:?}", tx_hash);
        let tx_bytes = tx.deref().to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();

        deployment_config.lq_tokens = Some(minted_token);
        write_deployment_to_disk(&deployment_config, config.deployment_json_path).await;
    }

    if deployment_config.splash_tokens.is_none() {
        println!("Minting SPLASH tokens ----------------------------------------------------------");

        let input_result = get_largest_utxo(&explorer, &addr).await;

        let (signed_tx_builder, minted_token) = mint_token::mint_token(
            SPLASH_NAME,
            2 * INFLATION_BOX_INITIAL_SPLASH_QTY,
            pk_hash,
            input_result,
            &addr,
            explorer.chain_tip_slot_number().await.unwrap(),
        );
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
        println!("tx_hash: {:?}", tx_hash);
        let tx_bytes = tx.deref().to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();

        deployment_config.splash_tokens = Some(minted_token);
        write_deployment_to_disk(&deployment_config, config.deployment_json_path).await;
    }

    let need_create_token_inputs = deployment_config.nft_utxo_inputs.is_none()
        || (deployment_config.minted_deployment_tokens.is_none()
            && deployment_config
                .nft_utxo_inputs
                .as_ref()
                .unwrap()
                .inputs_consumed);

    if need_create_token_inputs {
        println!("Creating inputs to mint deployment tokens ---------------------------------------");
        let input_result = get_largest_utxo(&explorer, &addr).await;
        println!("input ADA: {}", input_result.utxo_info.amount().coin);
        let signed_tx_builder = mint_token::create_minting_tx_inputs(input_result, &addr);
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
        println!("tx_hash: {:?}", tx_hash);
        let tx_bytes = tx.deref().to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();
        if let Some(ref mut i) = deployment_config.nft_utxo_inputs {
            i.tx_hash = tx_hash;
            i.inputs_consumed = false;
        } else {
            deployment_config.nft_utxo_inputs = Some(NFTUtxoInputs {
                tx_hash,
                number_of_inputs: mint_token::NUMBER_TOKEN_MINTS_NEEDED,
                inputs_consumed: false,
            });
            write_deployment_to_disk(&deployment_config, config.deployment_json_path).await;
        }
    }

    if deployment_config.minted_deployment_tokens.is_none() {
        println!("Minting deployment tokens ---------------------------------------");
        let mint_input_tx_hash = deployment_config.nft_utxo_inputs.as_ref().unwrap().tx_hash;
        let inputs = explorer
            .utxos_by_address(addr.clone(), 0, 100)
            .await
            .into_iter()
            .filter_map(|unspent_output| {
                if unspent_output.input.transaction_id == mint_input_tx_hash {
                    return Some(
                        SingleInputBuilder::new(unspent_output.input, unspent_output.output)
                            .payment_key()
                            .unwrap(),
                    );
                }
                None
            })
            .collect();
        let (signed_tx_builder, minted_tokens) =
            mint_token::mint_deployment_tokens(inputs, &addr, pk_hash, collateral.clone());
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
        println!("tx_hash: {:?}", tx_hash);
        let tx_bytes = tx.deref().to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();

        deployment_config.minted_deployment_tokens = Some(minted_tokens);
        deployment_config
            .nft_utxo_inputs
            .as_mut()
            .unwrap()
            .inputs_consumed = true;
        write_deployment_to_disk(&deployment_config, config.deployment_json_path).await;
    }
    if deployment_config.deployed_validators.is_none() {
        let time_source = NetworkTimeSource;
        let (tx_builder_0, tx_builder_1, tx_builder_2, reference_input_script_hashes) =
            mint_token::create_dao_reference_input_utxos(
                &deployment_config,
                time_source.network_time().await + dao_parameters.zeroth_epoch_start_offset,
            );

        println!("Creating reference inputs (batch #0) ---------------------------------------");
        let tx_hash_0 = deploy_dao_reference_inputs(tx_builder_0, &explorer, &addr, &prover).await;
        println!("Creating reference inputs (batch #1) ---------------------------------------");
        let tx_hash_1 = deploy_dao_reference_inputs(tx_builder_1, &explorer, &addr, &prover).await;
        println!("Creating reference inputs (batch #2) ---------------------------------------");
        let tx_hash_2 = deploy_dao_reference_inputs(tx_builder_2, &explorer, &addr, &prover).await;

        let make_ref_utxo = |batch_num: usize, output_index: u64| {
            if batch_num == 0 {
                ReferenceUTxO {
                    tx_hash: tx_hash_0,
                    output_index,
                }
            } else if batch_num == 1 {
                ReferenceUTxO {
                    tx_hash: tx_hash_1,
                    output_index,
                }
            } else {
                ReferenceUTxO {
                    tx_hash: tx_hash_2,
                    output_index,
                }
            }
        };
        // --------
        let d = DeployedValidators {
            inflation: DeployedValidatorRef {
                hash: reference_input_script_hashes.inflation,
                reference_utxo: make_ref_utxo(0, 0),
                cost: EX_UNITS,
                marginal_cost: None,
            },
            voting_escrow: DeployedValidatorRef {
                hash: reference_input_script_hashes.voting_escrow,
                reference_utxo: make_ref_utxo(0, 1),
                cost: EX_UNITS,
                marginal_cost: None,
            },
            farm_factory: DeployedValidatorRef {
                hash: reference_input_script_hashes.farm_factory,
                reference_utxo: make_ref_utxo(0, 2),
                cost: EX_UNITS,
                marginal_cost: None,
            },
            wp_factory: DeployedValidatorRef {
                hash: reference_input_script_hashes.wp_factory,
                reference_utxo: make_ref_utxo(0, 3),
                cost: EX_UNITS,
                marginal_cost: None,
            },
            ve_factory: DeployedValidatorRef {
                hash: reference_input_script_hashes.ve_factory,
                reference_utxo: make_ref_utxo(0, 4),
                cost: EX_UNITS,
                marginal_cost: None,
            },
            gov_proxy: DeployedValidatorRef {
                hash: reference_input_script_hashes.gov_proxy,
                reference_utxo: make_ref_utxo(1, 0),
                cost: EX_UNITS,
                marginal_cost: None,
            },
            perm_manager: DeployedValidatorRef {
                hash: reference_input_script_hashes.perm_manager,
                reference_utxo: make_ref_utxo(1, 1),
                cost: EX_UNITS,
                marginal_cost: None,
            },
            mint_wpauth_token: DeployedValidatorRef {
                hash: reference_input_script_hashes.mint_wpauth_token,
                reference_utxo: make_ref_utxo(1, 2),
                cost: EX_UNITS,
                marginal_cost: None,
            },
            mint_identifier: DeployedValidatorRef {
                hash: reference_input_script_hashes.mint_identifier,
                reference_utxo: make_ref_utxo(1, 3),
                cost: EX_UNITS,
                marginal_cost: None,
            },
            mint_ve_composition_token: DeployedValidatorRef {
                hash: reference_input_script_hashes.mint_ve_composition_token,
                reference_utxo: make_ref_utxo(2, 0),
                cost: EX_UNITS,
                marginal_cost: None,
            },
            weighting_power: DeployedValidatorRef {
                hash: reference_input_script_hashes.weighting_power,
                reference_utxo: make_ref_utxo(2, 1),
                cost: EX_UNITS,
                marginal_cost: None,
            },
            smart_farm: DeployedValidatorRef {
                hash: reference_input_script_hashes.smart_farm,
                reference_utxo: make_ref_utxo(2, 2),
                cost: EX_UNITS,
                marginal_cost: None,
            },
        };

        deployment_config.deployed_validators = Some(d);

        write_deployment_to_disk(&deployment_config, config.deployment_json_path).await;
    }

    let deployment_config = PreprodDeployment::try_from(deployment_config).unwrap();
    make_deposit(
        &explorer,
        &addr,
        collateral,
        &prover,
        &deployment_config,
        &dao_parameters,
    )
    .await;
    //create_dao_entities(
    //    &explorer,
    //    &addr,
    //    collateral,
    //    &prover,
    //    &deployment_config,
    //    dao_parameters,
    //)
    //.await;
    //create_initial_farms(&explorer, &addr, collateral, &prover, &deployment_config).await;
}

/// Note: need about 120 ADA to create these entities.
async fn deploy_dao_reference_inputs(
    mut tx_builder: TransactionBuilder,
    explorer: &Maestro,
    addr: &Address,
    prover: &OperatorProver,
) -> TransactionHash {
    let input_result = get_largest_utxo(explorer, addr).await;
    tx_builder.add_input(input_result).unwrap();
    let signed_tx_builder = tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap();
    let tx = prover.prove(signed_tx_builder);
    let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.deref().to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await.unwrap();
    explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();

    tx_hash
}

async fn create_dao_entities(
    explorer: &Maestro,
    addr: &Address,
    collateral: Collateral,
    prover: &OperatorProver,
    deployment_config: &PreprodDeployment,
    deployment_params: DaoDeploymentParameters,
) {
    let minted_tokens = &deployment_config.minted_deployment_tokens;
    let required_tokens = vec![
        minted_tokens.perm_auth.clone(),
        minted_tokens.ve_factory_auth.clone(),
        minted_tokens.gt.clone(),
        minted_tokens.factory_auth.clone(),
        minted_tokens.inflation_auth.clone(),
        minted_tokens.wp_factory_auth.clone(),
    ];
    let utxos = collect_utxos(addr, 5_000_000, required_tokens.clone(), &collateral, explorer).await;

    let mut tokens_in_inputs: HashMap<(ScriptHash, cml_chain::assets::AssetName), u64> = HashMap::default();
    for utxo in &utxos {
        for (policy_id, names) in utxo.utxo_info.amount().multiasset.iter() {
            for (name, quantity) in names.iter() {
                let qty = tokens_in_inputs
                    .entry((*policy_id, name.clone()))
                    .or_insert(0_u64);
                *qty += *quantity;
            }
        }
    }

    println!("# {} tokens in inputs", tokens_in_inputs.len());

    // Now remove tokens that will be placed into entities
    for BuiltPolicy {
        policy_id,
        asset_name,
        quantity,
        ..
    } in required_tokens
    {
        let mut remove = false;
        if let Some(qty) = tokens_in_inputs.get_mut(&(policy_id, asset_name.clone())) {
            println!("Removing {:?}", asset_name);
            *qty -= quantity.as_u64().unwrap();
            if *qty == 0 {
                remove = true;
            }
        }
        if remove {
            tokens_in_inputs.remove(&(policy_id, asset_name));
        }
    }

    println!("collect utxos: {:?}", utxos);
    let mut tx_builder = constant_tx_builder();

    let deployed_ref_inputs = &deployment_config.deployed_validators;
    let protocol_deployment = ProtocolDeployment::unsafe_pull(deployed_ref_inputs.clone(), explorer).await;
    tx_builder.add_reference_input(protocol_deployment.inflation.reference_utxo);
    tx_builder.add_reference_input(protocol_deployment.farm_factory.reference_utxo);
    tx_builder.add_reference_input(protocol_deployment.wp_factory.reference_utxo);
    tx_builder.add_reference_input(protocol_deployment.ve_factory.reference_utxo);
    tx_builder.add_reference_input(protocol_deployment.gov_proxy.reference_utxo);
    tx_builder.add_reference_input(protocol_deployment.perm_manager.reference_utxo);

    // Need to track input coin to
    let mut input_coin = 0;
    let mut output_coin = 0;

    for utxo in utxos {
        input_coin += utxo.utxo_info.amount().coin;
        tx_builder.add_input(utxo).unwrap();
    }

    let make_output = |script_hash: ScriptHash, datum: DatumOption, multi_asset: MultiAsset| {
        TransactionOutputBuilder::new()
            .with_address(script_address(script_hash, deployment_config.network_id))
            .with_data(datum)
            .next()
            .unwrap()
            .with_asset_and_min_required_coin(multi_asset, COINS_PER_UTXO_BYTE)
            .unwrap()
            .build()
            .unwrap()
    };

    // Inflation
    let mut inflation_assets = MultiAsset::default();
    inflation_assets.set(
        minted_tokens.inflation_auth.policy_id,
        minted_tokens.inflation_auth.asset_name.clone(),
        minted_tokens.inflation_auth.quantity.as_u64().unwrap(),
    );
    let inflation_out = make_output(
        protocol_deployment.inflation.hash,
        DatumOption::new_datum(PlutusData::new_integer(BigInteger::from(100_u64))),
        inflation_assets,
    );
    output_coin += inflation_out.output.value().coin;
    tx_builder.add_output(inflation_out).unwrap();

    // farm_factory
    let mut farm_assets = MultiAsset::default();
    farm_assets.set(
        minted_tokens.factory_auth.policy_id,
        minted_tokens.factory_auth.asset_name.clone(),
        minted_tokens.factory_auth.quantity.as_u64().unwrap(),
    );

    let farm_seed_data =
        PlutusData::new_bytes(protocol_deployment.perm_manager.hash.to_raw_bytes().to_vec()).to_cbor_bytes();
    let farm_factory_datum = FarmFactoryDatum {
        last_farm_id: -1,
        farm_seed_data,
    };
    let farm_factory_out = make_output(
        protocol_deployment.farm_factory.hash,
        DatumOption::new_datum(farm_factory_datum.into_pd()),
        farm_assets,
    );
    output_coin += farm_factory_out.output.value().coin;
    tx_builder.add_output(farm_factory_out).unwrap();

    // wp_factory
    let wp_factory_datum = PollFactoryConfig {
        last_poll_epoch: -1,
        active_farms: vec![],
    };
    let mut wp_factory_assets = MultiAsset::default();
    wp_factory_assets.set(
        minted_tokens.wp_factory_auth.policy_id,
        minted_tokens.wp_factory_auth.asset_name.clone(),
        minted_tokens.wp_factory_auth.quantity.as_u64().unwrap(),
    );
    let wp_factory_out = make_output(
        protocol_deployment.wp_factory.hash,
        DatumOption::new_datum(wp_factory_datum.into_pd()),
        wp_factory_assets,
    );
    output_coin += wp_factory_out.output.amount().coin;
    tx_builder.add_output(wp_factory_out).unwrap();

    let ve_factory_datum = VEFactoryDatum::from(deployment_params.accepted_assets);
    let mut ve_factory_assets = MultiAsset::default();
    ve_factory_assets.set(
        minted_tokens.ve_factory_auth.policy_id,
        minted_tokens.ve_factory_auth.asset_name.clone(),
        minted_tokens.ve_factory_auth.quantity.as_u64().unwrap(),
    );
    ve_factory_assets.set(
        minted_tokens.gt.policy_id,
        minted_tokens.gt.asset_name.clone(),
        minted_tokens.gt.quantity.as_u64().unwrap(),
    );
    let ve_factory_out = make_output(
        protocol_deployment.ve_factory.hash,
        DatumOption::new_datum(ve_factory_datum.into_pd()),
        ve_factory_assets,
    );
    output_coin += ve_factory_out.output.amount().coin;
    tx_builder.add_output(ve_factory_out).unwrap();

    let null_datum = DatumOption::new_datum(PlutusData::new_constr_plutus_data(ConstrPlutusData::new(
        0,
        vec![],
    )));

    // gov_proxy
    let gov_proxy_out = make_output(
        protocol_deployment.gov_proxy.hash,
        null_datum.clone(),
        MultiAsset::default(),
    );
    output_coin += gov_proxy_out.output.amount().coin;
    tx_builder.add_output(gov_proxy_out).unwrap();

    // perm_manager
    let mut perm_manager_assets = MultiAsset::default();
    perm_manager_assets.set(
        minted_tokens.perm_auth.policy_id,
        minted_tokens.perm_auth.asset_name.clone(),
        minted_tokens.perm_auth.quantity.as_u64().unwrap(),
    );

    let perm_manager_out = make_output(
        protocol_deployment.perm_manager.hash,
        null_datum,
        perm_manager_assets,
    );
    output_coin += perm_manager_out.output.amount().coin;
    tx_builder.add_output(perm_manager_out).unwrap();

    tx_builder
        .add_collateral(InputBuilderResult::from(collateral))
        .unwrap();

    println!("Creating DAO entities --------------------------------------------");
    let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
    let actual_fee = estimated_tx_fee + 200_000;
    tx_builder.set_fee(actual_fee);
    println!("Estimated fee: {}", estimated_tx_fee);

    // Adding change output ---------------------------------------------------
    let change_output_coin = input_coin - output_coin - actual_fee;
    println!("change output coin: {}", change_output_coin);
    println!("unused tokens: {:?}", tokens_in_inputs);
    let mut change_assets = MultiAsset::default();
    for ((policy_id, asset_name), quantity) in tokens_in_inputs {
        assert!(change_assets.set(policy_id, asset_name, quantity).is_none());
    }

    let change_output_output = TransactionOutputBuilder::new()
        .with_address(addr.clone())
        .next()
        .unwrap()
        .with_value(Value::new(change_output_coin, change_assets))
        .build()
        .unwrap();

    tx_builder.add_output(change_output_output).unwrap();

    let signed_tx_builder = tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap();
    let tx = prover.prove(signed_tx_builder);
    let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.deref().to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await.unwrap();
    explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();
}

async fn make_deposit(
    explorer: &Maestro,
    addr: &Address,
    collateral: Collateral,
    prover: &OperatorProver,
    deployment_config: &PreprodDeployment,
    deployment_params: &DaoDeploymentParameters,
) {
    let deployed_ref_inputs = &deployment_config.deployed_validators;
    let protocol_deployment = ProtocolDeployment::unsafe_pull(deployed_ref_inputs.clone(), explorer).await;

    let ve_factory_script_hash = protocol_deployment.ve_factory.hash;
    let network_id = deployment_config.network_id;

    let ve_factory_unspent_output = pull_onchain_entity::<VEFactorySnapshot, _>(
        explorer,
        ve_factory_script_hash,
        network_id,
        deployment_config,
    )
    .await
    .unwrap();
    let ve_factory_output_ref = OutputRef::new(
        ve_factory_unspent_output.input.transaction_id,
        ve_factory_unspent_output.input.index,
    );

    let ve_factory_in_value = ve_factory_unspent_output.output.amount();
    let mut ve_factory_out_value = ve_factory_in_value.clone();

    // Deposit assets into ve_factory -------------------------------------------
    let VEFactoryDatum { accepted_assets, .. } =
        VEFactoryDatum::from(deployment_params.accepted_assets.clone());

    let (token, _) = accepted_assets.first().unwrap();
    let ac = AssetClass::from(*token);
    ve_factory_out_value.add_unsafe(ac, 1_000_000);

    let ve_composition_policy = protocol_deployment.mint_ve_composition_token.hash;

    let (ve_composition_qty, mut voting_escrow_value) = exchange_outputs(
        ve_factory_in_value,
        &ve_factory_out_value,
        accepted_assets.clone(),
        ve_composition_policy,
        false,
    );

    // `ve_factory` will loan `ve_composition_qty` GT tokens to the newly created `voting_escrow`.
    let gt_token = &deployment_config.minted_deployment_tokens.gt;
    let gt_ac = AssetClass::from(Token(
        gt_token.policy_id,
        spectrum_cardano_lib::AssetName::from(gt_token.asset_name.clone()),
    ));
    ve_factory_out_value.sub_unsafe(gt_ac, ve_composition_qty);

    let bp = BuiltPolicy {
        policy_id: token.0,
        asset_name: cml_chain::assets::AssetName::from(token.1),
        quantity: BigInteger::from(1_000_000),
    };
    let utxos = collect_utxos(addr, 5_000_000, vec![bp], &collateral, explorer).await;

    #[derive(PartialEq, Eq)]
    enum InputType {
        VeFactory,
        Other,
    }

    // Need to find ve_factory's position within TX inputs.
    let mut output_refs: Vec<_> = utxos
        .iter()
        .map(|utxo| {
            let output_ref = OutputRef::new(utxo.input.transaction_id, utxo.input.index);
            (InputType::Other, output_ref)
        })
        .collect();

    output_refs.push((InputType::VeFactory, ve_factory_output_ref));

    output_refs.sort_by_key(|(_, output_ref)| *output_ref);
    let ve_factory_in_ix = output_refs
        .into_iter()
        .position(|(input_type, _)| input_type == InputType::VeFactory)
        .unwrap();

    let mut change_output_creator = ChangeOutputCreator::default();
    let mut tx_builder = constant_tx_builder();
    tx_builder.add_reference_input(protocol_deployment.ve_factory.reference_utxo);
    tx_builder.add_reference_input(protocol_deployment.voting_escrow.reference_utxo);
    tx_builder.add_reference_input(protocol_deployment.mint_ve_composition_token.reference_utxo);
    tx_builder.add_reference_input(protocol_deployment.mint_identifier.reference_utxo);

    // Add inputs --------------------------------------------------------
    for utxo in utxos {
        change_output_creator.add_input(&utxo);
        tx_builder.add_input(utxo).unwrap();
    }

    let ve_factory_redeemer = voting_escrow_factory::FactoryAction::Deposit.into_pd();

    let ve_factory_witness = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(ve_factory_script_hash),
        ve_factory_redeemer,
    );

    let ve_factory_input_builder = SingleInputBuilder::new(
        ve_factory_unspent_output.input,
        ve_factory_unspent_output.output.clone(),
    )
    .plutus_script_inline_datum(ve_factory_witness, vec![].into())
    .unwrap();
    change_output_creator.add_input(&ve_factory_input_builder);
    tx_builder.add_input(ve_factory_input_builder.clone()).unwrap();

    tx_builder.set_exunits(
        RedeemerWitnessKey::new(cml_chain::plutus::RedeemerTag::Spend, ve_factory_in_ix as u64),
        cml_chain::plutus::ExUnits::from(EX_UNITS_CREATE_VOTING_ESCROW),
    );

    let total_num_mints = voting_escrow_value.multiasset.len() + 1;

    // Mint ve_composition tokens --------------------------------------------
    let mint_ve_composition_token_witness = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(protocol_deployment.mint_ve_composition_token.hash),
        PlutusData::new_integer(BigInteger::from(ve_factory_in_ix)),
    );
    for (_, names) in voting_escrow_value.multiasset.iter() {
        for (asset_name, qty) in names.iter() {
            let mint_ve_composition_builder_result =
                SingleMintBuilder::new_single_asset(asset_name.clone(), *qty as i64)
                    .plutus_script(mint_ve_composition_token_witness.clone(), vec![].into());
            tx_builder.add_mint(mint_ve_composition_builder_result).unwrap();
        }
    }

    // NOW it is safe to add GT tokens to voting_escrow
    voting_escrow_value.add_unsafe(gt_ac, ve_composition_qty);

    // Mint ve_identifier token ------------------------------------------------
    let mint_ve_identifier_token_witness = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(protocol_deployment.mint_identifier.hash),
        ve_factory_output_ref.into_pd(),
    );
    println!("ve_factory_in output_ref: {}", ve_factory_output_ref);
    let mint_ve_identifier_name = compute_identifier_token_asset_name(ve_factory_output_ref);
    println!("identifier name: {}", mint_ve_identifier_name.to_raw_hex());
    let mint_ve_identifier_builder_result =
        SingleMintBuilder::new_single_asset(mint_ve_identifier_name.clone(), 1)
            .plutus_script(mint_ve_identifier_token_witness.clone(), vec![].into());
    tx_builder.add_mint(mint_ve_identifier_builder_result).unwrap();

    for ix in 0..total_num_mints {
        let ex_units = cml_chain::plutus::ExUnits::from(EX_UNITS);
        tx_builder.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Mint, ix as u64), ex_units);
    }

    let id_token = Token(
        protocol_deployment.mint_identifier.hash,
        spectrum_cardano_lib::AssetName::from(mint_ve_identifier_name),
    );
    voting_escrow_value.add_unsafe(AssetClass::from(id_token), 1);

    // Add outputs -----------------------------------------------------------------------
    let ve_factory_datum = if let Some(datum) = ve_factory_unspent_output.output.datum() {
        datum
    } else {
        panic!("farm_factory: expected datum!");
    };
    let ve_factory_output = TransactionOutputBuilder::new()
        .with_address(script_address(
            ve_factory_script_hash,
            deployment_config.network_id,
        ))
        .with_data(ve_factory_datum)
        .next()
        .unwrap()
        .with_asset_and_min_required_coin(ve_factory_out_value.multiasset, COINS_PER_UTXO_BYTE)
        .unwrap()
        .build()
        .unwrap();

    change_output_creator.add_output(&ve_factory_output);
    tx_builder.add_output(ve_factory_output).unwrap();

    let voting_escrow_datum = DatumOption::new_datum(
        VotingEscrowConfig {
            locked_until: Lock::Def((1728743924 + 10000) * 1000),
            owner: Owner::PubKey(vec![1, 2, 3]),
            max_ex_fee: 300_000,
            version: 0,
            last_wp_epoch: 0,
            last_gp_deadline: 0,
        }
        .into_pd(),
    );

    voting_escrow_value.coin = 2_000_000;

    let voting_escrow_output = TransactionOutputBuilder::new()
        .with_address(script_address(
            protocol_deployment.voting_escrow.hash,
            deployment_config.network_id,
        ))
        .with_data(voting_escrow_datum)
        .next()
        .unwrap()
        .with_value(voting_escrow_value)
        .build()
        .unwrap();
    change_output_creator.add_output(&voting_escrow_output);
    tx_builder.add_output(voting_escrow_output).unwrap();

    let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
    let actual_fee = estimated_tx_fee + 200_000;
    let change_output = change_output_creator.create_change_output(actual_fee, addr.clone());
    tx_builder.add_output(change_output).unwrap();
    tx_builder
        .add_collateral(InputBuilderResult::from(collateral))
        .unwrap();

    let start_slot = 73031120; //67580376;
    tx_builder.set_validity_start_interval(start_slot);
    tx_builder.set_ttl(start_slot + 43200);

    let signed_tx_builder = tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap();
    let tx = prover.prove(signed_tx_builder);
    let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.deref().to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await.unwrap();
    explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();
}

async fn create_initial_farms(
    explorer: &Maestro,
    addr: &Address,
    collateral: Collateral,
    prover: &OperatorProver,
    deployment_config: &PreprodDeploymentProgress,
) {
    let deployed_ref_inputs = deployment_config.deployed_validators.as_ref().unwrap();
    let protocol_deployment = ProtocolDeployment::unsafe_pull(deployed_ref_inputs.clone(), explorer).await;

    let minted_tokens = deployment_config.minted_deployment_tokens.as_ref().unwrap();
    let required_tokens = vec![minted_tokens.factory_auth.clone()];
    let utxos = collect_utxos(addr, 5_000_000, required_tokens.clone(), &collateral, explorer).await;

    let mut farm_factory_input = explorer
        .utxos_by_address(
            script_address(
                protocol_deployment.farm_factory.hash,
                deployment_config.network_id,
            ),
            0,
            10,
        )
        .await;

    assert_eq!(farm_factory_input.len(), 1);
    let farm_factory_unspent_output = farm_factory_input.pop().unwrap();

    let farm_factory_input_datum_pd =
        if let Some(DatumOption::Datum { datum, .. }) = farm_factory_unspent_output.output.datum() {
            datum
        } else {
            panic!("farm_factory: expected datum!");
        };

    #[derive(PartialEq, Eq)]
    enum InputType {
        FarmFactory,
        Other,
    }

    // Need to find farm_factory's position within TX inputs.
    let mut output_refs: Vec<_> = utxos
        .iter()
        .map(|utxo| {
            let output_ref = OutputRef::new(utxo.input.transaction_id, utxo.input.index);
            (InputType::Other, output_ref)
        })
        .collect();

    let farm_factory_output_ref = OutputRef::new(
        farm_factory_unspent_output.input.transaction_id,
        farm_factory_unspent_output.input.index,
    );
    output_refs.push((InputType::FarmFactory, farm_factory_output_ref));

    output_refs.sort_by_key(|(_, output_ref)| *output_ref);
    let farm_factory_in_ix = output_refs
        .into_iter()
        .position(|(input_type, _)| input_type == InputType::FarmFactory)
        .unwrap();

    println!("farm_factory_in_ix: {}", farm_factory_in_ix);
    let farm_factory_redeemer = FarmFactoryAction::CreateFarm.into_pd();
    let farm_factory_script_hash = protocol_deployment.farm_factory.hash;

    let farm_factory_witness = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(farm_factory_script_hash),
        farm_factory_redeemer,
    );

    let farm_factory_input_builder = SingleInputBuilder::new(
        farm_factory_unspent_output.input,
        farm_factory_unspent_output.output,
    )
    .plutus_script_inline_datum(farm_factory_witness, vec![].into())
    .unwrap();

    let mut change_output_creator = ChangeOutputCreator::default();

    let mut tx_builder = constant_tx_builder();
    tx_builder.add_reference_input(protocol_deployment.farm_factory.reference_utxo);
    tx_builder.add_reference_input(protocol_deployment.smart_farm.reference_utxo);
    for utxo in utxos {
        change_output_creator.add_input(&utxo);
        tx_builder.add_input(utxo).unwrap();
    }
    change_output_creator.add_input(&farm_factory_input_builder);
    tx_builder.add_input(farm_factory_input_builder.clone()).unwrap();

    tx_builder.set_exunits(
        RedeemerWitnessKey::new(cml_chain::plutus::RedeemerTag::Spend, farm_factory_in_ix as u64),
        cml_chain::plutus::ExUnits::from(EX_UNITS),
    );

    // Mint ------------------------------------------
    let mint_farm_auth_redeemer = MintAction::MintAuthToken {
        factory_in_ix: farm_factory_in_ix as u32,
    };
    let mint_farm_auth_witness = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(protocol_deployment.smart_farm.hash),
        mint_farm_auth_redeemer.into_pd(),
    );

    let farm_factory_in_datum = FarmFactoryDatum::try_from_pd(farm_factory_input_datum_pd).unwrap();
    let mint_farm_auth_asset_name = cml_chain::assets::AssetName::from_raw_bytes(
        &PlutusData::new_integer(BigInteger::from(farm_factory_in_datum.last_farm_id + 1)).to_cbor_bytes(),
    )
    .unwrap();

    let mint_farm_auth_builder_result =
        SingleMintBuilder::new_single_asset(mint_farm_auth_asset_name.clone(), 1)
            .plutus_script(mint_farm_auth_witness, vec![].into());
    tx_builder.add_mint(mint_farm_auth_builder_result).unwrap();
    let ex_units = cml_chain::plutus::ExUnits::from(EX_UNITS);
    tx_builder.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Mint, 0), ex_units);

    // farm_factory output ---------------------------------------
    let farm_factory_assets = farm_factory_input_builder.utxo_info.amount().multiasset.clone();
    let mut farm_factory_out_datum = farm_factory_in_datum.clone();
    farm_factory_out_datum.last_farm_id += 1;
    let farm_factory_output = TransactionOutputBuilder::new()
        .with_address(script_address(
            farm_factory_script_hash,
            deployment_config.network_id,
        ))
        .with_data(DatumOption::new_datum(farm_factory_out_datum.into_pd()))
        .next()
        .unwrap()
        .with_asset_and_min_required_coin(farm_factory_assets, COINS_PER_UTXO_BYTE)
        .unwrap()
        .build()
        .unwrap();

    change_output_creator.add_output(&farm_factory_output);
    tx_builder.add_output(farm_factory_output).unwrap();

    // smart_farm output ------------------------------------------
    let mut smart_farm_assets = MultiAsset::default();
    smart_farm_assets.set(protocol_deployment.smart_farm.hash, mint_farm_auth_asset_name, 1);
    let smart_farm_datum_pd =
        PlutusData::new_bytes(protocol_deployment.perm_manager.hash.to_raw_bytes().to_vec());
    println!(
        "smart_farm datum: {}",
        hex::encode(smart_farm_datum_pd.to_cbor_bytes())
    );
    let smart_farm_datum = DatumOption::new_datum(smart_farm_datum_pd);
    let smart_farm_output = TransactionOutputBuilder::new()
        .with_address(script_address(
            protocol_deployment.smart_farm.hash,
            deployment_config.network_id,
        ))
        .with_data(smart_farm_datum)
        .next()
        .unwrap()
        .with_asset_and_min_required_coin(smart_farm_assets, COINS_PER_UTXO_BYTE)
        .unwrap()
        .build()
        .unwrap();

    change_output_creator.add_output(&smart_farm_output);
    tx_builder.add_output(smart_farm_output).unwrap();

    tx_builder
        .add_collateral(InputBuilderResult::from(collateral))
        .unwrap();

    let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
    let actual_fee = estimated_tx_fee + 200_000;
    tx_builder.set_fee(actual_fee);
    println!("Estimated fee: {}", estimated_tx_fee);

    let change_output = change_output_creator.create_change_output(actual_fee, addr.clone());
    tx_builder.add_output(change_output).unwrap();

    let signed_tx_builder = tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap();
    let tx = prover.prove(signed_tx_builder);
    let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.deref().to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await.unwrap();
    explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();
}

pub async fn get_largest_utxo(explorer: &Maestro, addr: &Address) -> InputBuilderResult {
    let mut utxos = explorer.utxos_by_address(addr.clone(), 0, 50).await;
    utxos.sort_by_key(|output| output.output.value().coin);

    let utxo = utxos.last().cloned().unwrap();

    SingleInputBuilder::new(utxo.input, utxo.output)
        .payment_key()
        .unwrap()
}

async fn collect_utxos(
    addr: &Address,
    required_coin: Coin,
    required_tokens: Vec<BuiltPolicy>,
    collateral: &Collateral,
    explorer: &Maestro,
) -> Vec<InputBuilderResult> {
    if required_tokens.is_empty() {
        return collect_utxos_with_no_assets(addr, required_coin, collateral, explorer).await;
    }
    let mut res = vec![];
    let mut lovelaces_collected = 0;

    let mut skipped_utxos = vec![];

    // Maintains count of tokens whose required
    let mut count_satisfied_tokens = 0;

    let mut token_count = HashMap::new();

    let all_utxos = explorer.utxos_by_address(addr.clone(), 0, 100).await;
    for utxo in all_utxos {
        let output_ref = OutputRef::new(utxo.input.transaction_id, utxo.input.index);
        if output_ref != collateral.reference() {
            let mut add_utxo = false;
            if count_satisfied_tokens < required_tokens.len() {
                for (policy_id, name_map) in utxo.output.value().multiasset.iter() {
                    for (name, &quantity) in name_map.iter() {
                        if let Some(bp) = required_tokens
                            .iter()
                            .find(|bp| bp.policy_id == *policy_id && *name == bp.asset_name)
                        {
                            let quantity_collected = token_count.entry(output_ref).or_insert(0_u64);
                            let quantity_required = bp.quantity.as_u64().unwrap();
                            if *quantity_collected < quantity_required {
                                *quantity_collected += quantity;
                                add_utxo = true;
                                lovelaces_collected += utxo.output.amount().coin;
                                if *quantity_collected >= quantity_required {
                                    count_satisfied_tokens += 1;
                                }
                            }
                        }
                    }
                }
                if add_utxo {
                    let input_builder = SingleInputBuilder::new(utxo.input, utxo.output)
                        .payment_key()
                        .unwrap();
                    res.push(input_builder);
                } else {
                    skipped_utxos.push(utxo);
                }
            }
        }
    }

    if lovelaces_collected >= required_coin {
        return res;
    }

    // Here we've got all the required tokens but haven't met the required amount of lovelaces.
    // First sort UTxOs by coin, largest-to-smallest then select until target is met.
    skipped_utxos.sort_by_key(|u| u.output.amount().coin);
    while let Some(utxo) = skipped_utxos.pop() {
        lovelaces_collected += utxo.output.amount().coin;
        let input_builder = SingleInputBuilder::new(utxo.input, utxo.output)
            .payment_key()
            .unwrap();
        res.push(input_builder);
        if lovelaces_collected >= required_coin {
            break;
        }
    }

    res
}

async fn collect_utxos_with_no_assets(
    addr: &Address,
    required_coin: Coin,
    collateral: &Collateral,
    explorer: &Maestro,
) -> Vec<InputBuilderResult> {
    let mut res = vec![];
    let mut lovelaces_collected = 0;

    let mut all_utxos = explorer.utxos_by_address(addr.clone(), 0, 100).await;

    // We choose inputs with the fewest number of tokens and also the smallest ADA balances, to keep
    // the number of UTxOs in the wallet down.
    all_utxos.sort_by(|a, b| {
        let num_tokens_a = a.output.amount().multiasset.len();
        let num_tokens_b = b.output.amount().multiasset.len();
        if num_tokens_a < num_tokens_b {
            Ordering::Less
        } else if num_tokens_a == num_tokens_b {
            a.output.amount().coin.cmp(&b.output.amount().coin)
        } else {
            Ordering::Greater
        }
    });

    for utxo in all_utxos {
        let output_ref = OutputRef::new(utxo.input.transaction_id, utxo.input.index);
        if output_ref != collateral.reference() {
            if lovelaces_collected > required_coin {
                break;
            }

            let coin = utxo.output.amount().coin;
            lovelaces_collected += coin;
            let input_builder = SingleInputBuilder::new(utxo.input, utxo.output)
                .payment_key()
                .unwrap();
            res.push(input_builder);
        }
    }
    res
}

async fn pull_onchain_entity<'a, T, D>(
    explorer: &Maestro,
    script_hash: ScriptHash,
    network_id: NetworkId,
    deployment_config: &'a D,
) -> Option<TransactionUnspentOutput>
where
    T: TryFromLedger<TransactionOutput, DeploymentWithOutputRef<'a, D>>,
{
    let utxos = explorer
        .utxos_by_address(script_address(script_hash, network_id), 0, 50)
        .await;

    for utxo in utxos {
        let ve_factory_output_ref = OutputRef::from(utxo.clone().input);
        let ctx = DeploymentWithOutputRef {
            deployment: deployment_config,
            output_ref: ve_factory_output_ref,
        };
        if T::try_from_ledger(&utxo.output, &ctx).is_some() {
            return Some(utxo);
        }
    }
    None
}

fn compute_identifier_token_asset_name(output_ref: OutputRef) -> cml_chain::assets::AssetName {
    let mut bytes = output_ref.tx_hash().to_raw_bytes().to_vec();
    // let mut bytes = PlutusData::new_bytes(output_ref.tx_hash().to_raw_bytes().to_vec()).to_cbor_bytes();
    bytes.extend_from_slice(&PlutusData::new_integer(BigInteger::from(output_ref.index())).to_cbor_bytes());
    let token_name = blake2b256(bytes.as_ref());
    cml_chain::assets::AssetName::new(token_name.to_vec()).unwrap()
}

async fn write_deployment_to_disk(deployment_config: &PreprodDeploymentProgress, deployment_json_path: &str) {
    let mut file = tokio::fs::File::create(deployment_json_path).await.unwrap();
    file.write_all((serde_json::to_string(deployment_config).unwrap()).as_bytes())
        .await
        .unwrap();
}

#[derive(Parser)]
#[command(name = "splash-dao-administration")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Splash DAO Administration", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
}

#[derive(serde::Deserialize)]
#[serde(bound = "'de: 'a")]
pub struct AppConfig<'a> {
    pub network_id: NetworkId,
    pub maestro_key_path: &'a str,
    pub batcher_private_key: &'a str, //todo: store encrypted
    pub deployment_json_path: &'a str,
    pub parameters_json_path: &'a str,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PreprodDeploymentProgress {
    lq_tokens: Option<ExternallyMintedToken>,
    splash_tokens: Option<ExternallyMintedToken>,
    nft_utxo_inputs: Option<NFTUtxoInputs>,
    minted_deployment_tokens: Option<MintedTokens>,
    deployed_validators: Option<DeployedValidators>,
    network_id: NetworkId,
}

struct PreprodDeployment {
    lq_tokens: ExternallyMintedToken,
    splash_tokens: ExternallyMintedToken,
    nft_utxo_inputs: NFTUtxoInputs,
    minted_deployment_tokens: MintedTokens,
    deployed_validators: DeployedValidators,
    network_id: NetworkId,
}

impl Has<VEFactoryAuthPolicy> for PreprodDeployment {
    fn select<U: IsEqual<VEFactoryAuthPolicy>>(&self) -> VEFactoryAuthPolicy {
        VEFactoryAuthPolicy(self.minted_deployment_tokens.ve_factory_auth.policy_id)
    }
}

impl Has<GTAuthPolicy> for PreprodDeployment {
    fn select<U: IsEqual<GTAuthPolicy>>(&self) -> GTAuthPolicy {
        GTAuthPolicy(self.minted_deployment_tokens.gt.policy_id)
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::VeFactory as u8 }>> for PreprodDeployment {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::VeFactory as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::VeFactory as u8 }> {
        DeployedScriptInfo::from(&self.deployed_validators.ve_factory)
    }
}

trait NotOutputRef {}

impl NotOutputRef for VEFactoryAuthPolicy {}
impl NotOutputRef for GTAuthPolicy {}
impl<const TYP: u8> NotOutputRef for DeployedScriptInfo<TYP> {}

struct DeploymentWithOutputRef<'a, D> {
    deployment: &'a D,
    output_ref: OutputRef,
}

impl<'a, D> Has<OutputRef> for DeploymentWithOutputRef<'a, D> {
    fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.output_ref
    }
}

impl<'a, H, D> Has<H> for DeploymentWithOutputRef<'a, D>
where
    D: Has<H>,
    H: NotOutputRef,
{
    fn select<U: IsEqual<H>>(&self) -> H {
        self.deployment.select::<U>()
    }
}

impl TryFrom<PreprodDeploymentProgress> for PreprodDeployment {
    type Error = ();

    fn try_from(value: PreprodDeploymentProgress) -> Result<Self, Self::Error> {
        match value {
            PreprodDeploymentProgress {
                lq_tokens: Some(lq_tokens),
                splash_tokens: Some(splash_tokens),
                nft_utxo_inputs: Some(nft_utxo_inputs),
                minted_deployment_tokens: Some(minted_deployment_tokens),
                deployed_validators: Some(deployed_validators),
                network_id,
            } => Ok(Self {
                lq_tokens,
                splash_tokens,
                nft_utxo_inputs,
                minted_deployment_tokens,
                deployed_validators,
                network_id,
            }),
            _ => Err(()),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ExternallyMintedToken {
    policy_id: ScriptHash,
    asset_name: cml_chain::assets::AssetName,
    quantity: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
/// Each NFT we mint requires a distinct UTxO input.
struct NFTUtxoInputs {
    tx_hash: TransactionHash,
    number_of_inputs: usize,
    inputs_consumed: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct InitialDaoEntities {
    inflation_box: InflationBoxSnapshot,
    //farm_factory: FarmFactorySn
    weighting_poll_factory: PollFactorySnapshot,
    //voting_escrow_factory: VE
    // gov_proxy: Gov
    permission_manager: PermManagerSnapshot,
}

const LIMIT: u16 = 50;
const COLLATERAL_LOVELACES: u64 = 5_000_000;

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

async fn generate_collateral(
    explorer: &Maestro,
    addr: &Address,
    prover: &OperatorProver,
) -> Result<Collateral, Box<dyn std::error::Error>> {
    let input_utxo = get_largest_utxo(explorer, addr).await;
    let mut tx_builder = constant_tx_builder();
    tx_builder.add_input(input_utxo).unwrap();
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

const EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
};

const EX_UNITS_CREATE_VOTING_ESCROW: ExUnits = ExUnits {
    mem: 700_000,
    steps: 300_000_000,
};
