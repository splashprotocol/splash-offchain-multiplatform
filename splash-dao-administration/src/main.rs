mod mint_token;
mod user_simulator;
pub mod voting_order;

use std::{
    collections::HashMap,
    net::SocketAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use cardano_explorer::{CardanoNetwork, Maestro};
use clap::{command, Parser, Subcommand};
use cml_chain::{
    address::Address,
    assets::MultiAsset,
    builders::{
        input_builder::{InputBuilderResult, SingleInputBuilder},
        mint_builder::SingleMintBuilder,
        output_builder::TransactionOutputBuilder,
        redeemer_builder::RedeemerWitnessKey,
        tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionBuilder, TransactionUnspentOutput},
        withdrawal_builder::SingleWithdrawalBuilder,
        witness_builder::{PartialPlutusWitness, PlutusScriptWitness},
    },
    crypto::utils::make_vkey_witness,
    plutus::{ConstrPlutusData, PlutusData, RedeemerTag},
    transaction::{DatumOption, Transaction, TransactionOutput},
    utils::BigInteger,
    Coin, Serialize, Value,
};
use cml_crypto::{Ed25519KeyHash, PrivateKey, RawBytesEncoding, ScriptHash, TransactionHash};
use mint_token::{script_address, DaoDeploymentParameters, LQ_NAME};
use spectrum_cardano_lib::{
    collateral::Collateral,
    hash::hash_transaction_canonical,
    protocol_params::{constant_tx_builder, COINS_PER_UTXO_BYTE},
    transaction::TransactionOutputExtension,
    value::ValueExtension,
    AssetClass, NetworkId, OutputRef, PaymentCredential, Token,
};
use spectrum_cardano_lib::{plutus_data::IntoPlutusData, types::TryFromPData};
use spectrum_offchain::domain::Stable;
use spectrum_offchain::{domain::EntitySnapshot, ledger::TryFromLedger, tx_prover::TxProver};
use spectrum_offchain_cardano::{
    creds::operator_creds_base_address,
    deployment::{DeployedValidatorRef, ReferenceUTxO},
};
use splash_dao_offchain::{
    collateral::{pull_collateral, register_staking_address, send_assets},
    constants::{time::MAX_LOCK_TIME_SECONDS, DAO_SCRIPT_BYTES, SPLASH_NAME},
    create_change_output::{ChangeOutputCreator, CreateChangeOutput},
    deployment::{
        write_deployment_to_disk, BuiltPolicy, CompleteDeployment, DaoScriptData, DeployedValidators,
        DeploymentProgress, NFTUtxoInputs, ProtocolDeployment,
    },
    entities::{
        offchain::voting_order::VotingOrderId,
        onchain::{
            farm_factory::{FarmFactoryAction, FarmFactoryDatum},
            inflation_box::InflationBoxSnapshot,
            permission_manager::{PermManagerDatum, PermManagerSnapshot},
            poll_factory::{PollFactoryConfig, PollFactorySnapshot},
            smart_farm::{FarmId, MintAction},
            voting_escrow::{Lock, Owner, VotingEscrowConfig, VotingEscrowId, VotingEscrowSnapshot},
            voting_escrow_factory::{VEFactoryDatum, VEFactoryId, VEFactorySnapshot},
        },
    },
    routines::inflation::{actions::compute_farm_name, ProcessLedgerEntityContext, Slot, TimedOutputRef},
    time::NetworkTimeProvider,
    util::generate_collateral,
    CurrentEpoch, NetworkTimeSource,
};
use user_simulator::user_simulator;
use voting_order::create_voting_order;

const INFLATION_BOX_INITIAL_SPLASH_QTY: i64 = 32000000000000;

#[tokio::main]
async fn main() {
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    let dao_script_bytes_str =
        std::fs::read_to_string(args.script_bytes_path).expect("Cannot load script bytes file");
    let dao_script_bytes: DaoScriptData =
        serde_json::from_str(&dao_script_bytes_str).expect("Invalid script bytes file");

    DAO_SCRIPT_BYTES.set(dao_script_bytes).unwrap();

    let mut op_inputs = create_operation_inputs(&config).await;

    println!("Collateral output_ref: {}", op_inputs.collateral.reference());
    match args.command {
        Command::SimulateUser {
            ve_identifier_json_path,
            assets_json_path,
        } => {
            user_simulator(
                &mut op_inputs,
                config,
                &ve_identifier_json_path,
                &assets_json_path,
            )
            .await;
        }
        Command::SimulateUser {
            ve_identifier_json_path,
            assets_json_path,
        } => {
            user_simulator(
                &mut op_inputs,
                config,
                &ve_identifier_json_path,
                &assets_json_path,
            )
            .await;
        }
        Command::Deploy => {
            deploy(&mut op_inputs, config).await;
        }
        Command::CreateFarm => {
            create_initial_farms(&op_inputs).await;
        }
        Command::MakeVotingEscrow { assets_json_path } => {
            let s = std::fs::read_to_string(assets_json_path)
                .expect("Cannot load voting_escrow settings JSON file");
            let ve_settings: VotingEscrowSettings =
                serde_json::from_str(&s).expect("Invalid voting_escrow settings file");
            make_voting_escrow_order(&ve_settings, &mut op_inputs).await;
        }
        Command::ExtendDeposit => {
            //
        }
        Command::CastVote { ve_identifier_hex } => {
            let id = VotingEscrowId::from(
                spectrum_cardano_lib::AssetName::try_from_hex(&ve_identifier_hex).unwrap(),
            );
            cast_vote(&op_inputs, id).await;
        }

        Command::SendEDaoToken { destination_addr } => {
            send_edao_token(&op_inputs, destination_addr).await;
        }
        Command::RegisterWitnessStakingAddress { script_hash_hex } => {
            register_witness_staking_addr(&op_inputs, script_hash_hex).await;
        }
    };
}

async fn deploy<'a>(op_inputs: &mut OperationInputs, config: AppConfig<'a>) -> CompleteDeployment {
    let OperationInputs {
        explorer,
        addr,
        collateral,
        prover,
        operator_public_key_hash,
        deployment_progress,
        dao_parameters,
        network_id,
        ..
    } = op_inputs;

    if let Ok(deployment_config) = CompleteDeployment::try_from((deployment_progress.clone(), *network_id)) {
        return deployment_config;
    }

    let mut deployment_progress = deployment_progress.clone();

    println!("Collateral output_ref: {}", collateral.reference());

    let operator_pkh_str: String = operator_public_key_hash.clone().into();

    let pk_hash = Ed25519KeyHash::from_bech32(&operator_pkh_str).unwrap();

    let input_result = get_largest_utxo(explorer, addr).await;
    println!(
        "input_result: {}#{}, value: {:?}",
        input_result.input.transaction_id.to_hex(),
        input_result.input.index,
        input_result.utxo_info.value(),
    );

    //let raw_deployment_config =
    //    std::fs::read_to_string(config.deployment_json_path).expect("Cannot load configuration file");
    //let mut deployment_config: DeploymentProgress =
    //    serde_json::from_str(&raw_deployment_config).expect("Invalid configuration file");

    // Mint LQ token -------------------------------------------------------------------------------
    if deployment_progress.lq_tokens.is_none() {
        println!("Minting LQ tokens ----------------------------------------------------------");
        let (signed_tx_builder, minted_token) = mint_token::mint_token(
            LQ_NAME,
            INFLATION_BOX_INITIAL_SPLASH_QTY,
            pk_hash,
            input_result,
            addr,
            explorer.chain_tip_slot_number().await.unwrap(),
        );
        let tx = Transaction::from(prover.prove(signed_tx_builder));
        let tx_hash = TransactionHash::from_hex(&tx.body.hash().to_hex()).unwrap();
        println!("tx_hash: {}", tx_hash.to_hex());
        let tx_bytes = tx.to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();

        deployment_progress.lq_tokens = Some(minted_token);
        write_deployment_to_disk(&deployment_progress, config.deployment_json_path).await;
    }

    if deployment_progress.splash_tokens.is_none() {
        println!("Minting SPLASH tokens ----------------------------------------------------------");

        let input_result = get_largest_utxo(explorer, addr).await;

        let (signed_tx_builder, minted_token) = mint_token::mint_token(
            SPLASH_NAME,
            2 * INFLATION_BOX_INITIAL_SPLASH_QTY,
            pk_hash,
            input_result,
            addr,
            explorer.chain_tip_slot_number().await.unwrap(),
        );
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.body.hash().to_hex()).unwrap();
        println!("tx_hash: {}", tx_hash.to_hex());
        let tx_bytes = tx.to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();

        deployment_progress.splash_tokens = Some(minted_token);
        write_deployment_to_disk(&deployment_progress, config.deployment_json_path).await;
    }

    let need_create_token_inputs = deployment_progress.nft_utxo_inputs.is_none()
        || (deployment_progress.minted_deployment_tokens.is_none()
            && deployment_progress
                .nft_utxo_inputs
                .as_ref()
                .unwrap()
                .inputs_consumed);

    if need_create_token_inputs {
        println!("Creating inputs to mint deployment tokens ---------------------------------------");
        let input_result = get_largest_utxo(explorer, addr).await;
        println!("input ADA: {}", input_result.utxo_info.amount().coin);
        let signed_tx_builder = mint_token::create_minting_tx_inputs(input_result, addr);
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.body.hash().to_hex()).unwrap();
        println!("tx_hash: {:?}", tx_hash);
        let tx_bytes = tx.to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();
        if let Some(ref mut i) = deployment_progress.nft_utxo_inputs {
            i.tx_hash = tx_hash;
            i.inputs_consumed = false;
        } else {
            deployment_progress.nft_utxo_inputs = Some(NFTUtxoInputs {
                tx_hash,
                number_of_inputs: mint_token::NUMBER_TOKEN_MINTS_NEEDED,
                inputs_consumed: false,
            });
            write_deployment_to_disk(&deployment_progress, config.deployment_json_path).await;
        }
    }

    if deployment_progress.minted_deployment_tokens.is_none() {
        println!("Minting deployment tokens ---------------------------------------");
        let mint_input_tx_hash = deployment_progress.nft_utxo_inputs.as_ref().unwrap().tx_hash;
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
            mint_token::mint_deployment_tokens(inputs, addr, pk_hash, collateral.clone());
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.body.hash().to_hex()).unwrap();
        println!("tx_hash: {:?}", tx_hash);
        let tx_bytes = tx.to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();

        deployment_progress.minted_deployment_tokens = Some(minted_tokens);
        deployment_progress
            .nft_utxo_inputs
            .as_mut()
            .unwrap()
            .inputs_consumed = true;
        write_deployment_to_disk(&deployment_progress, config.deployment_json_path).await;
    }
    if deployment_progress.deployed_validators.is_none() {
        let time_source = NetworkTimeSource;
        let genesis_epoch_start_time =
            (time_source.network_time().await - dao_parameters.zeroth_epoch_start_offset) * 1000;
        let (tx_builder_0, tx_builder_1, tx_builder_2, reference_input_script_hashes) =
            mint_token::create_dao_reference_input_utxos(
                &deployment_progress,
                genesis_epoch_start_time,
                config.network_id,
            );

        println!("Creating reference inputs (batch #0) ---------------------------------------");
        let tx_hash_0 = deploy_dao_reference_inputs(tx_builder_0, explorer, addr, prover).await;
        println!("Creating reference inputs (batch #1) ---------------------------------------");
        let tx_hash_1 = deploy_dao_reference_inputs(tx_builder_1, explorer, addr, prover).await;
        println!("Creating reference inputs (batch #2) ---------------------------------------");
        let tx_hash_2 = deploy_dao_reference_inputs(tx_builder_2, explorer, addr, prover).await;

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
        let dsd = DaoScriptData::global();
        let d = DeployedValidators {
            inflation: DeployedValidatorRef {
                hash: reference_input_script_hashes.inflation,
                reference_utxo: make_ref_utxo(0, 0),
                cost: (&dsd.inflation.ex_units).into(),
                marginal_cost: None,
            },
            voting_escrow: DeployedValidatorRef {
                hash: reference_input_script_hashes.voting_escrow,
                reference_utxo: make_ref_utxo(0, 1),
                cost: (&dsd.voting_escrow.ex_units).into(),
                marginal_cost: None,
            },
            farm_factory: DeployedValidatorRef {
                hash: reference_input_script_hashes.farm_factory,
                reference_utxo: make_ref_utxo(0, 2),
                cost: (&dsd.farm_factory.ex_units).into(),
                marginal_cost: None,
            },
            wp_factory: DeployedValidatorRef {
                hash: reference_input_script_hashes.wp_factory,
                reference_utxo: make_ref_utxo(0, 3),
                cost: (&dsd.wp_factory.ex_units).into(),
                marginal_cost: None,
            },
            ve_factory: DeployedValidatorRef {
                hash: reference_input_script_hashes.ve_factory,
                reference_utxo: make_ref_utxo(0, 4),
                cost: (&dsd.ve_factory.ex_units).into(),
                marginal_cost: None,
            },
            gov_proxy: DeployedValidatorRef {
                hash: reference_input_script_hashes.gov_proxy,
                reference_utxo: make_ref_utxo(1, 0),
                cost: (&dsd.gov_proxy.ex_units).into(),
                marginal_cost: None,
            },
            perm_manager: DeployedValidatorRef {
                hash: reference_input_script_hashes.perm_manager,
                reference_utxo: make_ref_utxo(1, 1),
                cost: (&dsd.perm_manager.ex_units).into(),
                marginal_cost: None,
            },
            mint_wpauth_token: DeployedValidatorRef {
                hash: reference_input_script_hashes.mint_wpauth_token,
                reference_utxo: make_ref_utxo(1, 2),
                cost: (&dsd.mint_wp_auth_token.mint_ex_units).into(),
                marginal_cost: None,
            },
            mint_identifier: DeployedValidatorRef {
                hash: reference_input_script_hashes.mint_identifier,
                reference_utxo: make_ref_utxo(1, 3),
                cost: (&dsd.mint_identifier.ex_units).into(),
                marginal_cost: None,
            },
            mint_ve_composition_token: DeployedValidatorRef {
                hash: reference_input_script_hashes.mint_ve_composition_token,
                reference_utxo: make_ref_utxo(2, 0),
                cost: (&dsd.mint_ve_composition_token.ex_units).into(),
                marginal_cost: None,
            },
            weighting_power: DeployedValidatorRef {
                hash: reference_input_script_hashes.weighting_power,
                reference_utxo: make_ref_utxo(2, 1),
                cost: (&dsd.mint_weighting_power.mint_ex_units).into(),
                marginal_cost: None,
            },
            smart_farm: DeployedValidatorRef {
                hash: reference_input_script_hashes.smart_farm,
                reference_utxo: make_ref_utxo(2, 2),
                cost: (&dsd.mint_farm_auth_token.ex_units).into(),
                marginal_cost: None,
            },
            make_ve_order: DeployedValidatorRef {
                hash: reference_input_script_hashes.make_ve_order,
                reference_utxo: make_ref_utxo(2, 3),
                cost: (&dsd.make_voting_escrow_order.ex_units).into(),
                marginal_cost: None,
            },
        };

        deployment_progress.deployed_validators = Some(d);
        deployment_progress.genesis_epoch_start_time = Some(genesis_epoch_start_time);

        write_deployment_to_disk(&deployment_progress, config.deployment_json_path).await;
    }

    let deployment_config = CompleteDeployment::try_from((deployment_progress.clone(), *network_id))
        .expect(INCOMPLETE_DEPLOYMENT_ERR_MSG);
    create_dao_entities(
        explorer,
        addr,
        collateral.clone(),
        prover,
        config.network_id,
        &deployment_config,
        dao_parameters,
    )
    .await;

    op_inputs.deployment_progress = deployment_progress;

    for _ in 0..deployment_config.num_initial_farms {
        create_initial_farms(op_inputs).await;
    }
    deployment_config
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
    let tx_hash = TransactionHash::from_hex(&tx.body.hash().to_hex()).unwrap();
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.to_cbor_bytes();
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
    network_id: NetworkId,
    deployment_config: &CompleteDeployment,
    deployment_params: &DaoDeploymentParameters,
) {
    let minted_tokens = &deployment_config.minted_deployment_tokens;
    let splash_built_policy = BuiltPolicy {
        policy_id: deployment_config.splash_tokens.policy_id,
        asset_name: deployment_config.splash_tokens.asset_name.clone(),
        quantity: BigInteger::from(deployment_config.splash_tokens.quantity),
    };
    let required_tokens = vec![
        minted_tokens.perm_auth.clone(),
        minted_tokens.ve_factory_auth.clone(),
        minted_tokens.gt.clone(),
        minted_tokens.factory_auth.clone(),
        minted_tokens.inflation_auth.clone(),
        minted_tokens.wp_factory_auth.clone(),
        splash_built_policy,
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
            .with_address(script_address(script_hash, network_id))
            .with_data(datum)
            .next()
            .unwrap()
            .with_asset_and_min_required_coin(multi_asset, COINS_PER_UTXO_BYTE)
            .unwrap()
            .build()
            .unwrap()
    };

    // Inflation -----------------------------------------------------------------------------------
    let mut inflation_assets = MultiAsset::default();
    inflation_assets.set(
        minted_tokens.inflation_auth.policy_id,
        minted_tokens.inflation_auth.asset_name.clone(),
        minted_tokens.inflation_auth.quantity.as_u64().unwrap(),
    );
    inflation_assets.set(
        deployment_config.splash_tokens.policy_id,
        deployment_config.splash_tokens.asset_name.clone(),
        deployment_config.splash_tokens.quantity,
    );
    let inflation_out = make_output(
        protocol_deployment.inflation.hash,
        DatumOption::new_datum(PlutusData::new_integer(BigInteger::from(0_u64))),
        inflation_assets,
    );
    output_coin += inflation_out.output.value().coin;
    tx_builder.add_output(inflation_out).unwrap();

    // farm_factory --------------------------------------------------------------------------------
    let mut farm_assets = MultiAsset::default();
    farm_assets.set(
        minted_tokens.factory_auth.policy_id,
        minted_tokens.factory_auth.asset_name.clone(),
        minted_tokens.factory_auth.quantity.as_u64().unwrap(),
    );

    let farm_seed_data =
        PlutusData::new_bytes(minted_tokens.perm_auth.policy_id.to_raw_bytes().to_vec()).to_cbor_bytes();
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

    // wp_factory ----------------------------------------------------------------------------------

    let active_farms = (0..deployment_params.num_active_farms)
        .map(|farm_id| {
            let name = spectrum_cardano_lib::AssetName::from(compute_farm_name(farm_id));
            FarmId(name)
        })
        .collect();

    let wp_factory_datum = PollFactoryConfig {
        last_poll_epoch: -1,
        active_farms,
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

    let ve_factory_datum = VEFactoryDatum::from(deployment_params.accepted_assets.clone());
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

    // gov_proxy -----------------------------------------------------------------------------------
    let gov_proxy_out = make_output(
        protocol_deployment.gov_proxy.hash,
        null_datum.clone(),
        MultiAsset::default(),
    );
    output_coin += gov_proxy_out.output.amount().coin;
    tx_builder.add_output(gov_proxy_out).unwrap();

    // perm_manager --------------------------------------------------------------------------------
    let mut perm_manager_assets = MultiAsset::default();
    perm_manager_assets.set(
        minted_tokens.perm_auth.policy_id,
        minted_tokens.perm_auth.asset_name.clone(),
        minted_tokens.perm_auth.quantity.as_u64().unwrap(),
    );

    let perm_manager_datum = PermManagerDatum {
        authorized_executors: deployment_params.authorized_executors.clone(),
        suspended_farms: vec![],
    };

    let perm_manager_out = make_output(
        protocol_deployment.perm_manager.hash,
        DatumOption::new_datum(perm_manager_datum.into_pd()),
        perm_manager_assets,
    );
    output_coin += perm_manager_out.output.amount().coin;
    tx_builder.add_output(perm_manager_out).unwrap();

    tx_builder
        .add_collateral(InputBuilderResult::from(collateral))
        .unwrap();

    println!("Creating DAO entities --------------------------------------------");
    let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
    let actual_fee = estimated_tx_fee + 300_000;
    tx_builder.set_fee(actual_fee);
    println!("Estimated fee: {}", estimated_tx_fee);

    // Adding change output ------------------------------------------------------------------------
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
    let tx_hash = TransactionHash::from_hex(&tx.body.hash().to_hex()).unwrap();
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await.unwrap();
    explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();
    println!("TX confirmed");
}

async fn make_voting_escrow_order(
    ve_settings: &VotingEscrowSettings,
    op_inputs: &mut OperationInputs,
) -> Owner {
    let OperationInputs {
        explorer,
        addr,
        owner_pub_key,
        deployment_progress,
        dao_parameters,
        collateral,
        prover,
        network_id,
        ..
    } = op_inputs;

    let VotingEscrowSettings {
        deposits,
        max_ex_fee,
        ada_balance,
        lock_duration_in_seconds,
        ..
    } = ve_settings;

    if *lock_duration_in_seconds > MAX_LOCK_TIME_SECONDS {
        panic!("Locktime exceeds limits!");
    }

    let deployment_config = CompleteDeployment::try_from((deployment_progress.clone(), *network_id))
        .expect(INCOMPLETE_DEPLOYMENT_ERR_MSG);
    let protocol_deployment =
        ProtocolDeployment::unsafe_pull(deployment_config.deployed_validators.clone(), explorer).await;

    let mut order_out_value = Value::zero();

    // Deposit assets into ve_factory -------------------------------------------
    let VEFactoryDatum { accepted_assets, .. } = VEFactoryDatum::from(dao_parameters.accepted_assets.clone());

    let mut built_policies = vec![];

    for token_deposit in deposits {
        let token = Token::from(token_deposit);
        let accepted_asset = accepted_assets.iter().any(|(tok, _)| *tok == token);
        let ac = AssetClass::from(token);
        if accepted_asset {
            order_out_value.add_unsafe(ac, token_deposit.quantity);
            built_policies.push(BuiltPolicy::from(token_deposit.clone()));
        } else {
            panic!("{} is not accepted by the DAO!", ac);
        }
    }

    println!("seeking input value: {}", ada_balance + 10_000_000);
    let utxos = collect_utxos(
        addr,
        ada_balance + 10_000_000,
        built_policies,
        collateral,
        explorer,
    )
    .await;

    let mut change_output_creator = ChangeOutputCreator::default();
    let mut tx_builder = constant_tx_builder();
    tx_builder.add_reference_input(protocol_deployment.make_ve_order.reference_utxo);

    // Add inputs --------------------------------------------------------
    for utxo in utxos {
        println!("add_input coin: {}", utxo.utxo_info.value().coin);
        change_output_creator.add_input(&utxo);
        tx_builder.add_input(utxo).unwrap();
    }

    let time_source = NetworkTimeSource;
    let locked_until = Lock::Def((time_source.network_time().await + *lock_duration_in_seconds) * 1000);
    let owner_bytes = owner_pub_key.to_raw_bytes().try_into().unwrap();
    let owner = Owner::PubKey(owner_bytes);
    let voting_escrow_datum = DatumOption::new_datum(
        VotingEscrowConfig {
            locked_until,
            owner,
            max_ex_fee: *max_ex_fee as u32,
            version: 0,
            last_wp_epoch: -1,
            last_gp_deadline: -1,
        }
        .into_pd(),
    );

    order_out_value.coin = *ada_balance;

    let mve_output = TransactionOutputBuilder::new()
        .with_address(script_address(
            protocol_deployment.make_ve_order.hash,
            *network_id,
        ))
        .with_data(voting_escrow_datum)
        .next()
        .unwrap()
        .with_value(order_out_value)
        .build()
        .unwrap();
    change_output_creator.add_output(&mve_output);
    tx_builder.add_output(mve_output).unwrap();

    let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
    let actual_fee = estimated_tx_fee + 300_000;
    let change_output = change_output_creator.create_change_output(actual_fee, addr.clone());
    tx_builder.add_output(change_output).unwrap();
    tx_builder
        .add_collateral(InputBuilderResult::from(collateral.clone()))
        .unwrap();

    let start_slot = explorer.chain_tip_slot_number().await.unwrap();
    tx_builder.set_validity_start_interval(start_slot);
    tx_builder.set_ttl(start_slot + 300);

    let signed_tx_builder = tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap();
    let tx = prover.prove(signed_tx_builder);
    let tx_hash = TransactionHash::from_hex(&tx.body.hash().to_hex()).unwrap();
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await.unwrap();
    explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();
    println!("TX confirmed");
    owner
}

async fn create_initial_farms(op_inputs: &OperationInputs) {
    let OperationInputs {
        explorer,
        addr,
        deployment_progress,
        collateral,
        prover,
        network_id,
        ..
    } = op_inputs;

    let deployment_config = CompleteDeployment::try_from((deployment_progress.clone(), *network_id))
        .expect(INCOMPLETE_DEPLOYMENT_ERR_MSG);

    let deployed_ref_inputs = &deployment_config.deployed_validators;
    let protocol_deployment = ProtocolDeployment::unsafe_pull(deployed_ref_inputs.clone(), explorer).await;

    let minted_tokens = &deployment_config.minted_deployment_tokens;
    let required_tokens = vec![minted_tokens.factory_auth.clone()];
    let utxos = collect_utxos(addr, 5_000_000, required_tokens.clone(), collateral, explorer).await;

    let mut farm_factory_input = explorer
        .utxos_by_address(
            script_address(protocol_deployment.farm_factory.hash, *network_id),
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
        DaoScriptData::global().farm_factory.ex_units.clone(),
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
    tx_builder.set_exunits(
        RedeemerWitnessKey::new(RedeemerTag::Mint, 0),
        DaoScriptData::global().mint_farm_auth_token.ex_units.clone(),
    );

    // farm_factory output ---------------------------------------
    let farm_factory_assets = farm_factory_input_builder.utxo_info.amount().multiasset.clone();
    let mut farm_factory_out_datum = farm_factory_in_datum.clone();
    farm_factory_out_datum.last_farm_id += 1;
    let farm_factory_output = TransactionOutputBuilder::new()
        .with_address(script_address(farm_factory_script_hash, *network_id))
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
    let smart_farm_datum_pd = PlutusData::new_bytes(
        deployment_config
            .minted_deployment_tokens
            .perm_auth
            .policy_id
            .to_raw_bytes()
            .to_vec(),
    );
    println!(
        "smart_farm datum: {}",
        hex::encode(smart_farm_datum_pd.to_cbor_bytes())
    );
    let smart_farm_datum = DatumOption::new_datum(smart_farm_datum_pd);
    let mut smart_farm_output = TransactionOutputBuilder::new()
        .with_address(script_address(protocol_deployment.smart_farm.hash, *network_id))
        .with_data(smart_farm_datum)
        .next()
        .unwrap()
        .with_asset_and_min_required_coin(smart_farm_assets, COINS_PER_UTXO_BYTE)
        .unwrap()
        .build()
        .unwrap();

    // Need to increase ADA here since the farm will take splash on inflation distribution.
    smart_farm_output.output.add_asset(AssetClass::Native, 250_000);

    change_output_creator.add_output(&smart_farm_output);
    tx_builder.add_output(smart_farm_output).unwrap();

    tx_builder
        .add_collateral(InputBuilderResult::from(collateral.clone()))
        .unwrap();

    let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
    let actual_fee = estimated_tx_fee + 400_000;
    tx_builder.set_fee(actual_fee);
    println!("Estimated fee: {}", estimated_tx_fee);

    let change_output = change_output_creator.create_change_output(actual_fee, addr.clone());
    tx_builder.add_output(change_output).unwrap();

    let signed_tx_builder = tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap();
    let tx = prover.prove(signed_tx_builder);
    let tx_hash = TransactionHash::from_hex(&tx.body.hash().to_hex()).unwrap();
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await.unwrap();
    explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();
}

async fn cast_vote(op_inputs: &OperationInputs, id: VotingEscrowId) {
    let OperationInputs {
        voting_order_listener_endpoint,
        operator_sk,
        deployment_progress,
        explorer,
        network_id,
        dao_parameters,
        ..
    } = op_inputs;

    let deployment_config = CompleteDeployment::try_from((deployment_progress.clone(), *network_id))
        .expect(INCOMPLETE_DEPLOYMENT_ERR_MSG);
    let protocol_deployment =
        ProtocolDeployment::unsafe_pull(deployment_config.deployed_validators.clone(), explorer).await;

    let voting_escrow_script_hash = protocol_deployment.voting_escrow.hash;

    let (voting_escrow, _voting_escrow_unspent_output) = pull_onchain_entity::<VotingEscrowSnapshot, _>(
        explorer,
        voting_escrow_script_hash,
        *network_id,
        &deployment_config,
        id,
    )
    .await
    .unwrap();

    let id = VotingOrderId {
        voting_escrow_id: voting_escrow.get().stable_id(),
        version: voting_escrow.get().version as u64,
    };

    let current_posix_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let voting_power = voting_escrow.get().voting_power(current_posix_time);

    let wpoll_policy_id = deployment_config.deployed_validators.mint_wpauth_token.hash;
    let voting_order = create_voting_order(
        operator_sk,
        id,
        voting_power,
        wpoll_policy_id,
        0,
        dao_parameters.num_active_farms,
    );

    let client = reqwest::Client::new();

    let url = format!(
        "http://{}{}",
        voting_order_listener_endpoint, "/submit/votingorder"
    );

    // Send the PUT request with JSON body
    let response = client
        .put(url)
        .json(&voting_order) // Serialize the payload as JSON
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let text = response.text().await.unwrap();
        println!("Response: {}", text);
    } else {
        println!("Failed with status: {}", response.status());
        let error_text = response.text().await.unwrap();
        println!("Error: {}", error_text);
    }
}

pub async fn get_largest_utxo<Net: CardanoNetwork>(explorer: &Net, addr: &Address) -> InputBuilderResult {
    let mut utxos = explorer.utxos_by_address(addr.clone(), 0, 100).await;
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
    let all_utxos = explorer.utxos_by_address(addr.clone(), 0, 100).await;
    splash_dao_offchain::collect_utxos::collect_utxos(
        all_utxos,
        required_coin,
        required_tokens,
        Some(collateral),
    )
}

const LIMIT: u16 = 50;
async fn pull_onchain_entity<'a, T, D>(
    explorer: &Maestro,
    script_hash: ScriptHash,
    network_id: NetworkId,
    deployment_config: &'a D,
    id: T::StableId,
) -> Option<(T, TransactionUnspentOutput)>
where
    T: TryFromLedger<TransactionOutput, ProcessLedgerEntityContext<'a, D>> + EntitySnapshot,
{
    let mut entity = None;
    let mut offset = 0u64;

    while entity.is_none() {
        let utxos = explorer
            .slot_indexed_utxos_by_address(script_address(script_hash, network_id), offset as u32, LIMIT)
            .await;
        if utxos.is_empty() {
            break;
        }

        println!("pulled utxos from slot {}: # pulled: {}", offset, utxos.len(),);

        for (utxo, slot) in utxos {
            let timed_output_ref = TimedOutputRef {
                output_ref: OutputRef::from(utxo.clone().input),
                slot: Slot(slot),
            };
            let ctx = ProcessLedgerEntityContext {
                behaviour: deployment_config,
                timed_output_ref,
                current_epoch: CurrentEpoch::from(0),
                wpoll_eliminated: false,
            };
            if let Some(t) = T::try_from_ledger(&utxo.output, &ctx) {
                println!("  ID: {}, slot: {}", t.stable_id(), slot);
                if t.stable_id() == id {
                    entity = Some((t, utxo));
                }
                if offset < slot {
                    offset = slot + 1;
                }
            }
        }
    }
    entity
}

async fn send_edao_token(op_inputs: &OperationInputs, destination_addr: String) {
    let OperationInputs {
        explorer,
        addr,
        deployment_progress,
        prover,
        network_id,
        ..
    } = op_inputs;
    let destination_addr = Address::from_bech32(&destination_addr).unwrap();
    let deployment_config = CompleteDeployment::try_from((deployment_progress.clone(), *network_id))
        .expect(INCOMPLETE_DEPLOYMENT_ERR_MSG);

    let minted_tokens = &deployment_config.minted_deployment_tokens;
    let required_tokens = vec![minted_tokens.edao_msig.clone()];
    send_assets(
        5_000_000,
        1_800_000,
        vec![],
        explorer,
        addr,
        &destination_addr,
        prover,
    )
    .await
    .unwrap();
}

async fn register_witness_staking_addr(op_inputs: &OperationInputs, script_hash_hex: String) {
    let OperationInputs {
        explorer,
        addr,
        prover,
        collateral,
        ..
    } = op_inputs;
    let staking_validator_script_hash = ScriptHash::from_hex(&script_hash_hex).unwrap();
    //println!(
    //    "ZZZZ: {}",
    //    hex::encode(vec![
    //        158, 118, 55, 184, 13, 29, 242, 39, 236, 32, 97, 168, 142, 119, 32, 223, 131, 28, 159, 233, 162,
    //        22, 58, 3, 52, 9, 157, 158
    //    ])
    //);
    register_staking_address(
        14_000_000,
        8_030_000,
        explorer,
        addr,
        &staking_validator_script_hash,
        collateral,
        prover,
    )
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
    /// Path to the JSON DAO script bytes file.
    #[arg(long, short)]
    script_bytes_path: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Deploy,
    CreateFarm,
    MakeVotingEscrow {
        #[arg(long)]
        assets_json_path: String,
    },
    ExtendDeposit,
    CastVote {
        #[arg(long)]
        ve_identifier_hex: String,
    },
    SendEDaoToken {
        #[arg(long)]
        destination_addr: String,
    },
    SimulateUser {
        #[arg(long)]
        ve_identifier_json_path: String,
        #[arg(long)]
        assets_json_path: String,
    },
    RegisterWitnessStakingAddress {
        #[arg(long)]
        script_hash_hex: String,
    },
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct VotingEscrowSettings {
    deposits: Vec<TokenDeposit>,
    max_ex_fee: u64,
    ada_balance: u64,
    lock_duration_in_seconds: u64,
    creation_epoch: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct TokenDeposit {
    pub asset_name_utf8: String,
    pub policy_id: ScriptHash,
    pub quantity: u64,
}

impl From<TokenDeposit> for BuiltPolicy {
    fn from(value: TokenDeposit) -> Self {
        let asset_name = cml_chain::assets::AssetName::try_from(&*value.asset_name_utf8).unwrap();
        Self {
            policy_id: value.policy_id,
            asset_name,
            quantity: BigInteger::from(value.quantity),
        }
    }
}

impl From<&TokenDeposit> for Token {
    fn from(value: &TokenDeposit) -> Self {
        let asset_name = spectrum_cardano_lib::AssetName::utf8_unsafe(value.asset_name_utf8.clone());
        Token(value.policy_id, asset_name)
    }
}

#[derive(serde::Deserialize)]
#[serde(bound = "'de: 'a")]
pub struct AppConfig<'a> {
    pub network_id: NetworkId,
    pub maestro_key_path: &'a str,
    pub batcher_private_key: &'a str, //todo: store encrypted
    pub deployment_json_path: &'a str,
    pub parameters_json_path: &'a str,
    pub voting_order_listener_endpoint: SocketAddr,
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

struct OperationInputs {
    explorer: Maestro,
    addr: Address,
    owner_pub_key: cml_crypto::PublicKey,
    operator_public_key_hash: PaymentCredential,
    operator_sk: PrivateKey,
    deployment_progress: DeploymentProgress,
    dao_parameters: DaoDeploymentParameters,
    collateral: Collateral,
    prover: OperatorProver,
    network_id: NetworkId,
    voting_order_listener_endpoint: SocketAddr,
}

async fn create_operation_inputs<'a>(config: &'a AppConfig<'a>) -> OperationInputs {
    let explorer = Maestro::new(config.maestro_key_path, config.network_id.into())
        .await
        .expect("Maestro instantiation failed");

    let (addr, _, operator_pkh, _operator_cred, operator_sk) =
        operator_creds_base_address(config.batcher_private_key, config.network_id);
    let sk_bech32 = operator_sk.to_bech32();
    let prover = OperatorProver::new(sk_bech32);
    //let prover = OperatorProver::new(config.batcher_private_key.into());
    let owner_pub_key = operator_sk.to_public();

    let collateral = if let Some(c) = pull_collateral(addr.clone().into(), &explorer).await {
        c
    } else {
        generate_collateral(&explorer, &addr, &addr, &prover)
            .await
            .unwrap()
    };

    let dao_parameters_str =
        std::fs::read_to_string(config.parameters_json_path).expect("Cannot load dao parameters file");
    let dao_parameters: DaoDeploymentParameters =
        serde_json::from_str(&dao_parameters_str).expect("Invalid parameters file");

    let raw_deployment_config =
        std::fs::read_to_string(config.deployment_json_path).expect("Cannot load configuration file");
    let deployment_progress: DeploymentProgress =
        serde_json::from_str(&raw_deployment_config).expect("Invalid configuration file");

    OperationInputs {
        explorer,
        addr,
        owner_pub_key,
        operator_public_key_hash: operator_pkh,
        operator_sk,
        collateral,
        dao_parameters,
        deployment_progress,
        prover,
        network_id: config.network_id,
        voting_order_listener_endpoint: config.voting_order_listener_endpoint,
    }
}

const INCOMPLETE_DEPLOYMENT_ERR_MSG: &str =
    "Expected a complete deployment! Run `splash-dao-administration deploy` to complete deployment";

#[derive(Clone)]
pub struct OperatorProver(String);

impl OperatorProver {
    pub fn new(sk_bech32: String) -> Self {
        Self(sk_bech32)
    }
}

impl TxProver<SignedTxBuilder, Transaction> for OperatorProver {
    fn prove(&self, mut candidate: SignedTxBuilder) -> Transaction {
        let body = candidate.body();
        let tx_hash = hash_transaction_canonical(&body);
        let sk = PrivateKey::from_bech32(self.0.as_str()).unwrap();
        let signature = make_vkey_witness(&tx_hash, &sk);
        candidate.add_vkey(signature);
        match candidate.build_checked() {
            Ok(tx) => tx,
            Err(err) => panic!("CML returned error: {}", err),
        }
    }
}
