mod mint_token;
pub mod voting_order;

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    hash::Hash,
    net::SocketAddr,
    ops::Deref,
    time::{SystemTime, UNIX_EPOCH},
};

use cardano_explorer::{CardanoNetwork, Maestro};
use clap::{command, Parser, Subcommand};
use cml_chain::{
    address::Address,
    assets::{self, MultiAsset},
    builders::{
        input_builder::{InputBuilderResult, SingleInputBuilder},
        mint_builder::SingleMintBuilder,
        output_builder::TransactionOutputBuilder,
        redeemer_builder::RedeemerWitnessKey,
        tx_builder::{ChangeSelectionAlgo, TransactionBuilder, TransactionUnspentOutput},
        withdrawal_builder::SingleWithdrawalBuilder,
        witness_builder::{PartialPlutusWitness, PlutusScriptWitness},
    },
    certs::Credential,
    plutus::{ConstrPlutusData, PlutusData, PlutusScript, PlutusV2Script, RedeemerTag},
    transaction::{DatumOption, TransactionOutput},
    utils::BigInteger,
    Coin, Serialize, Value,
};
use cml_crypto::{blake2b256, Ed25519KeyHash, PrivateKey, RawBytesEncoding, ScriptHash, TransactionHash};
use mint_token::{script_address, DaoDeploymentParameters, LQ_NAME};
use spectrum_cardano_lib::{
    collateral::Collateral,
    ex_units::ExUnits,
    protocol_params::{constant_tx_builder, COINS_PER_UTXO_BYTE},
    transaction::TransactionOutputExtension,
    value::ValueExtension,
    AssetClass, NetworkId, OutputRef, PaymentCredential, Token,
};
use spectrum_cardano_lib::{plutus_data::IntoPlutusData, types::TryFromPData};
use spectrum_offchain::data::Stable;
use spectrum_offchain::{
    data::{EntitySnapshot, Has},
    ledger::TryFromLedger,
    tx_prover::TxProver,
};
use spectrum_offchain_cardano::{
    creds::{operator_creds, operator_creds_base_address, CollateralAddress},
    deployment::{DeployedScriptInfo, DeployedValidatorRef, ReferenceUTxO},
    prover::operator::OperatorProver,
};
use splash_dao_offchain::{
    collateral::{pull_collateral, send_assets},
    constants::{script_bytes::VOTING_WITNESS_STUB, DEFAULT_AUTH_TOKEN_NAME, SPLASH_NAME},
    create_change_output::{ChangeOutputCreator, CreateChangeOutput},
    deployment::{
        write_deployment_to_disk, BuiltPolicy, CompleteDeployment, DeployedValidators, DeploymentProgress,
        MintedTokens, NFTUtxoInputs, ProtocolDeployment, ProtocolValidator,
    },
    entities::{
        offchain::voting_order::VotingOrderId,
        onchain::{
            farm_factory::{FarmFactoryAction, FarmFactoryDatum},
            inflation_box::InflationBoxSnapshot,
            permission_manager::{PermManagerDatum, PermManagerSnapshot},
            poll_factory::{PollFactoryConfig, PollFactorySnapshot},
            smart_farm::{FarmId, MintAction},
            voting_escrow::{
                Lock, Owner, VotingEscrowAction, VotingEscrowAuthorizedAction, VotingEscrowConfig,
                VotingEscrowId, VotingEscrowSnapshot,
            },
            voting_escrow_factory::{self, exchange_outputs, VEFactoryDatum, VEFactoryId, VEFactorySnapshot},
        },
    },
    protocol_config::{GTAuthPolicy, NotOutputRefNorSlotNumber, VEFactoryAuthPolicy},
    routines::inflation::{actions::compute_farm_name, ProcessLedgerEntityContext, Slot, TimedOutputRef},
    time::NetworkTimeProvider,
    util::generate_collateral,
    CurrentEpoch, NetworkTimeSource,
};
use tokio::io::AsyncWriteExt;
use type_equalities::IsEqual;
use uplc_pallas_traverse::output;
use voting_order::create_voting_order;

const INFLATION_BOX_INITIAL_SPLASH_QTY: i64 = 32000000000000;

#[tokio::main]
async fn main() {
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    let op_inputs = create_operation_inputs(&config).await;

    println!("Collateral output_ref: {}", op_inputs.collateral.reference());
    match args.command {
        Command::Deploy => {
            deploy(op_inputs, config).await;
        }
        Command::CreateFarm => {
            create_initial_farms(&op_inputs).await;
        }
        Command::MakeVotingEscrow { assets_json_path } => {
            let s = std::fs::read_to_string(assets_json_path)
                .expect("Cannot load voting_escrow settings JSON file");
            let ve_settings: VotingEscrowSettings =
                serde_json::from_str(&s).expect("Invalid voting_escrow settings file");
            make_voting_escrow(ve_settings, &op_inputs).await;
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
    };
}

async fn deploy<'a>(op_inputs: OperationInputs, config: AppConfig<'a>) {
    let OperationInputs {
        explorer,
        addr,
        owner_pub_key,
        collateral,
        prover,
        operator_public_key_hash,
        mut deployment_progress,
        dao_parameters,
        ..
    } = op_inputs;

    println!("Collateral output_ref: {}", collateral.reference());

    let operator_pkh_str: String = operator_public_key_hash.into();

    let pk_hash = Ed25519KeyHash::from_bech32(&operator_pkh_str).unwrap();

    let input_result = get_largest_utxo(&explorer, &addr).await;

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

        deployment_progress.lq_tokens = Some(minted_token);
        write_deployment_to_disk(&deployment_progress, config.deployment_json_path).await;
    }

    if deployment_progress.splash_tokens.is_none() {
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
            mint_token::mint_deployment_tokens(inputs, &addr, pk_hash, collateral.clone());
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
        println!("tx_hash: {:?}", tx_hash);
        let tx_bytes = tx.deref().to_cbor_bytes();
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

        deployment_progress.deployed_validators = Some(d);
        deployment_progress.genesis_epoch_start_time = Some(genesis_epoch_start_time);

        write_deployment_to_disk(&deployment_progress, config.deployment_json_path).await;
    }

    let deployment_config =
        CompleteDeployment::try_from(deployment_progress).expect(INCOMPLETE_DEPLOYMENT_ERR_MSG);
    create_dao_entities(
        &explorer,
        &addr,
        collateral.clone(),
        &prover,
        config.network_id,
        &deployment_config,
        &dao_parameters,
    )
    .await;
    //make_deposit(
    //    &explorer,
    //    &addr,
    //    owner_pub_key,
    //    collateral,
    //    &prover,
    //    config.network_id,
    //    &deployment_config,
    //    &dao_parameters,
    //)
    //.await;
    //create_initial_farms(
    //    &explorer,
    //    &addr,
    //    collateral,
    //    &prover,
    //    config.network_id,
    //    &deployment_config,
    //)
    //.await;
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
    let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.deref().to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await.unwrap();
    explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();
    println!("TX confirmed");
}

async fn make_voting_escrow(ve_settings: VotingEscrowSettings, op_inputs: &OperationInputs) {
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
    } = ve_settings;

    let deployment_config =
        CompleteDeployment::try_from(deployment_progress.clone()).expect(INCOMPLETE_DEPLOYMENT_ERR_MSG);
    let protocol_deployment =
        ProtocolDeployment::unsafe_pull(deployment_config.deployed_validators.clone(), explorer).await;

    let ve_factory_script_hash = protocol_deployment.ve_factory.hash;

    let (ve_factory, ve_factory_unspent_output) = pull_onchain_entity::<VEFactorySnapshot, _>(
        explorer,
        ve_factory_script_hash,
        *network_id,
        &deployment_config,
        VEFactoryId,
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
    let VEFactoryDatum { accepted_assets, .. } = VEFactoryDatum::from(dao_parameters.accepted_assets.clone());

    let mut built_policies = vec![];

    for token_deposit in deposits {
        let token = Token::from(&token_deposit);
        let accepted_asset = accepted_assets.iter().any(|(tok, _)| *tok == token);
        let ac = AssetClass::from(token);
        if accepted_asset {
            ve_factory_out_value.add_unsafe(ac, token_deposit.quantity);
            built_policies.push(BuiltPolicy::from(token_deposit));
        } else {
            panic!("{} is not accepted by the DAO!", ac);
        }
    }

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

    let utxos = collect_utxos(addr, 5_000_000, built_policies, collateral, explorer).await;

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
        .with_address(script_address(ve_factory_script_hash, *network_id))
        .with_data(ve_factory_datum)
        .next()
        .unwrap()
        .with_asset_and_min_required_coin(ve_factory_out_value.multiasset, COINS_PER_UTXO_BYTE)
        .unwrap()
        .build()
        .unwrap();

    change_output_creator.add_output(&ve_factory_output);
    tx_builder.add_output(ve_factory_output).unwrap();

    let time_source = NetworkTimeSource;
    const ONE_MONTH_IN_SECONDS: u64 = 604800 * 4;
    let voting_escrow_datum = DatumOption::new_datum(
        VotingEscrowConfig {
            locked_until: Lock::Def((time_source.network_time().await + lock_duration_in_seconds) * 1000),
            owner: Owner::PubKey(owner_pub_key.to_raw_bytes().to_vec()),
            max_ex_fee: max_ex_fee as u32,
            version: 0,
            last_wp_epoch: -1,
            last_gp_deadline: -1,
        }
        .into_pd(),
    );

    voting_escrow_value.coin = ada_balance;

    let voting_escrow_output = TransactionOutputBuilder::new()
        .with_address(script_address(
            protocol_deployment.voting_escrow.hash,
            *network_id,
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
        .add_collateral(InputBuilderResult::from(collateral.clone()))
        .unwrap();

    let start_slot = explorer.chain_tip_slot_number().await.unwrap();
    tx_builder.set_validity_start_interval(start_slot);
    tx_builder.set_ttl(start_slot + 300);

    let signed_tx_builder = tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap();
    let tx = prover.prove(signed_tx_builder);
    let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.deref().to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await.unwrap();
    explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();
    println!("TX confirmed");
}

async fn create_initial_farms(op_inputs: &OperationInputs) {
    let OperationInputs {
        explorer,
        addr,
        owner_pub_key,
        operator_public_key_hash,
        deployment_progress,
        dao_parameters,
        collateral,
        prover,
        network_id,
        ..
    } = op_inputs;

    let deployment_config =
        CompleteDeployment::try_from(deployment_progress.clone()).expect(INCOMPLETE_DEPLOYMENT_ERR_MSG);

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

    let deployment_config =
        CompleteDeployment::try_from(deployment_progress.clone()).expect(INCOMPLETE_DEPLOYMENT_ERR_MSG);
    let protocol_deployment =
        ProtocolDeployment::unsafe_pull(deployment_config.deployed_validators.clone(), explorer).await;

    let voting_escrow_script_hash = protocol_deployment.voting_escrow.hash;

    let (voting_escrow, voting_escrow_unspent_output) = pull_onchain_entity::<VotingEscrowSnapshot, _>(
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

    let voting_order = create_voting_order(operator_sk, id, voting_power, dao_parameters.num_active_farms);

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
    let utxos = explorer
        .slot_indexed_utxos_by_address(script_address(script_hash, network_id), 0, 50)
        .await;

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
            return Some((t, utxo));
        }
    }
    None
}

async fn send_edao_token(op_inputs: &OperationInputs, destination_addr: String) {
    let OperationInputs {
        explorer,
        addr,
        deployment_progress,
        prover,
        ..
    } = op_inputs;
    let destination_addr = Address::from_bech32(&destination_addr).unwrap();
    let deployment_config =
        CompleteDeployment::try_from(deployment_progress.clone()).expect(INCOMPLETE_DEPLOYMENT_ERR_MSG);

    let minted_tokens = &deployment_config.minted_deployment_tokens;
    let required_tokens = vec![minted_tokens.edao_msig.clone()];
    send_assets(required_tokens, explorer, addr, &destination_addr, prover)
        .await
        .unwrap();
}

fn compute_identifier_token_asset_name(output_ref: OutputRef) -> cml_chain::assets::AssetName {
    let mut bytes = output_ref.tx_hash().to_raw_bytes().to_vec();
    bytes.extend_from_slice(&PlutusData::new_integer(BigInteger::from(output_ref.index())).to_cbor_bytes());
    let token_name = blake2b256(bytes.as_ref());
    cml_chain::assets::AssetName::new(token_name.to_vec()).unwrap()
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
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct VotingEscrowSettings {
    deposits: Vec<TokenDeposit>,
    max_ex_fee: u64,
    ada_balance: u64,
    lock_duration_in_seconds: u64,
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

const EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
};

const EX_UNITS_CREATE_VOTING_ESCROW: ExUnits = ExUnits {
    mem: 700_000,
    steps: 300_000_000,
};

const INCOMPLETE_DEPLOYMENT_ERR_MSG: &str =
    "Expected a complete deployment! Run `splash-dao-administration deploy` to complete deployment";
