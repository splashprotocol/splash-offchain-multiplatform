use cardano_explorer::retry;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{PlutusScript, PlutusV2Script, PlutusV3Script};
use cml_chain::PolicyId;
use cml_crypto::PrivateKey;
use cml_crypto::RawBytesEncoding;
use futures_timer::Delay;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::OutputRef;
use splash_dao_offchain::entities::offchain::ExtendVotingEscrowOffChainOrder;
use splash_dao_offchain::entities::offchain::OffChainOrderId;
use splash_dao_offchain::entities::offchain::RedeemVotingEscrowOffChainOrder;
use splash_dao_offchain::entities::onchain::redeem_voting_escrow::make_redeem_ve_witness_redeemer;
use splash_dao_offchain::entities::onchain::voting_escrow::Lock;
use splash_dao_offchain::entities::onchain::voting_escrow_factory::VEFactoryId;
use splash_dao_offchain::entities::onchain::voting_escrow_factory::VEFactorySnapshot;
use splash_dao_offchain::{
    deployment::{CompleteDeployment, DaoScriptData, ProtocolDeployment},
    entities::{
        offchain::{compute_voting_escrow_witness_message, voting_order::VotingOrder},
        onchain::{
            extend_voting_escrow_order::{
                compute_extend_ve_witness_validator, make_extend_ve_witness_redeemer,
                ExtendVotingEscrowOnchainOrder, ExtendVotingEscrowOrderAction,
            },
            make_voting_escrow_order::DaoOrderStatus,
            voting_escrow::{Owner, VotingEscrowId, VotingEscrowSnapshot},
            weighting_poll::{WeightingPollId, WeightingPollSnapshot},
        },
    },
    routines::inflation::time_millis_to_epoch,
    time::epoch_start,
    CurrentEpoch, GenesisEpochStartTime,
};
use std::{
    net::SocketAddr,
    time::{Duration, SystemTime},
};
use tokio::io::AsyncWriteExt;

use crate::{
    deploy, extend_voting_escrow_order, make_voting_escrow_order, pull_onchain_entity,
    voting_order::create_voting_order, AppConfig, OperationInputs, VotingEscrowSettings,
};

const EPOCH_WAIT_TIME: u64 = 30_000;

pub async fn user_simulator<'a>(
    op_inputs: &mut OperationInputs,
    config: AppConfig<'a>,
    ve_identifier_json_path: &str,
    assets_json_path: &str,
) {
    // 1. deploy
    let deployment_config = deploy(op_inputs, config).await;

    let owner_bytes = op_inputs.owner_pub_key.to_raw_bytes().try_into().unwrap();
    let owner = Owner::PubKey(owner_bytes);
    let protocol_deployment =
        ProtocolDeployment::unsafe_pull(deployment_config.deployed_validators.clone(), &op_inputs.explorer)
            .await;

    let s = std::fs::read_to_string(assets_json_path).expect("Cannot load voting_escrow settings JSON file");
    let ve_settings: VotingEscrowSettings =
        serde_json::from_str(&s).expect("Invalid voting_escrow settings file");

    // Current epoch as measured determined by weighting_poll
    let mut wpoll_current_epoch = None;
    let mut ve_state = VEState::Waiting(owner);

    let genesis_epoch_time = GenesisEpochStartTime::from(deployment_config.genesis_epoch_start_time);
    loop {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let CurrentEpoch(current_epoch) = time_millis_to_epoch(now, genesis_epoch_time);

        let ve_identifier_str = tokio::fs::read_to_string(ve_identifier_json_path)
            .await
            .expect("Cannot load dao parameters file");
        let user_ve_identifier: UserVEIdentifier =
            serde_json::from_str(&ve_identifier_str).expect("Invalid ve_identifiers file");

        // Create initial voting_escrow
        if ve_settings.creation_epoch == current_epoch && user_ve_identifier.identifier_name.is_none() {
            assert!(matches!(ve_state, VEState::Waiting(_)));
            println!("---- Making deposit into VE");
            // Deposit into VE
            let owner = make_voting_escrow_order(&ve_settings, op_inputs).await;

            let voting_escrow_id = loop {
                if let Some(mve_status) =
                    request_mve_status(owner, &op_inputs.voting_order_listener_endpoint).await
                {
                    match mve_status {
                        DaoOrderStatus::Unspent => {
                            println!("MVE order not yet processed by bot");
                        }
                        DaoOrderStatus::Refunded => {
                            panic!("Unexpected refund of MVE order!");
                        }
                        DaoOrderStatus::SpentToFormVotingEscrow(voting_escrow_id) => {
                            break voting_escrow_id;
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            };
            let identifier_name = cml_chain::assets::AssetName::from(voting_escrow_id.0);
            if let Some((ve_snapshot, _)) = pull_onchain_entity::<VotingEscrowSnapshot, _>(
                &op_inputs.explorer,
                protocol_deployment.voting_escrow.hash,
                op_inputs.network_id,
                &deployment_config,
                voting_escrow_id,
            )
            .await
            {
                ve_state = VEState::ConfirmedVotingEscrow(ve_snapshot, Epoch(current_epoch));
            }
            let mut file = tokio::fs::File::create(ve_identifier_json_path).await.unwrap();
            file.write_all(
                (serde_json::to_string(&UserVEIdentifier {
                    identifier_name: Some(identifier_name),
                })
                .unwrap())
                .as_bytes(),
            )
            .await
            .unwrap();

            continue;
        }

        let try_pull_next_wpoll = (wpoll_current_epoch.is_none()
            || wpoll_current_epoch.unwrap() < current_epoch)
            && now - epoch_start(genesis_epoch_time, current_epoch) > EPOCH_WAIT_TIME;

        if try_pull_next_wpoll {
            println!("try pull WPOLL");
            let pulled = pull_onchain_entity::<WeightingPollSnapshot, _>(
                &op_inputs.explorer,
                protocol_deployment.mint_wpauth_token.hash,
                op_inputs.network_id,
                &deployment_config,
                WeightingPollId(current_epoch),
            )
            .await
            .is_some();
            if pulled {
                println!("Pulled WPOLL for epoch {}", current_epoch);
                wpoll_current_epoch = Some(current_epoch);
            } else {
                println!("FAILED pull WPOLL");
            }
        }

        let pulled_current_epoch =
            wpoll_current_epoch.is_some() && wpoll_current_epoch.unwrap() == current_epoch;

        if pulled_current_epoch {
            if let Some(ve_id) = user_ve_identifier.identifier_name {
                let voting_escrow_id = VotingEscrowId(spectrum_cardano_lib::AssetName::from(ve_id.clone()));
                match ve_state {
                    VEState::PredictedVoteCast(e) => {
                        if let Some((ve_snapshot, _)) = pull_onchain_entity::<VotingEscrowSnapshot, _>(
                            &op_inputs.explorer,
                            protocol_deployment.voting_escrow.hash,
                            op_inputs.network_id,
                            &deployment_config,
                            voting_escrow_id,
                        )
                        .await
                        {
                            let last_wp_epoch = ve_snapshot.get().last_wp_epoch;
                            if current_epoch as i32 == last_wp_epoch {
                                println!("Vote confirmed for epoch {}", current_epoch);
                                ve_state = VEState::ConfirmedVoteCast {
                                    ve_snapshot,
                                    ve_extended_this_epoch: false,
                                };
                            }
                        }
                    }
                    VEState::ConfirmedVoteCast {
                        ref ve_snapshot,
                        ve_extended_this_epoch,
                    } => {
                        // wait until epoch's ended (or add funds, vote on proposal)
                        let version = ve_snapshot.get().version as u64;
                        let last_wp_epoch = ve_snapshot.get().last_wp_epoch;
                        if last_wp_epoch < current_epoch as i32 {
                            println!(
                                "Voting from state {:?} in epoch {}, version: {}",
                                ve_state, current_epoch, version
                            );
                            let voting_power = ve_snapshot.get().voting_power(now);
                            let wpoll_policy_id =
                                deployment_config.deployed_validators.mint_wpauth_token.hash;
                            let voting_order_id = OffChainOrderId {
                                voting_escrow_id,
                                version,
                            };
                            let voting_order = create_voting_order(
                                &op_inputs.operator_sk,
                                voting_order_id,
                                voting_power,
                                wpoll_policy_id,
                                current_epoch,
                                op_inputs.dao_parameters.num_active_farms,
                            );

                            send_vote(voting_order, &op_inputs.voting_order_listener_endpoint).await;

                            ve_state = VEState::PredictedVoteCast(Epoch(current_epoch));
                        } else if !ve_extended_this_epoch {
                            let owner =
                                extend_voting_escrow_order(voting_escrow_id, &ve_settings, op_inputs).await;
                            let ve_output_ref = ve_snapshot.version().output_ref;
                            ve_state = VEState::PredictedOnChainExtendedVE(
                                Epoch(current_epoch),
                                owner,
                                ve_output_ref,
                                VEVersion(version),
                            );
                        } else {
                        }
                    }
                    VEState::PredictedOnChainExtendedVE(e, owner, ve_output_ref, VEVersion(version)) => {
                        // If order UTxO is found we can then send off-chain order to the bot.
                        if let Some((_, output)) = pull_onchain_entity::<ExtendVotingEscrowOnchainOrder, _>(
                            &op_inputs.explorer,
                            protocol_deployment.extend_ve_order.hash,
                            op_inputs.network_id,
                            &deployment_config,
                            owner,
                        )
                        .await
                        {
                            enum T {
                                Order,
                                VE,
                                VEFactory,
                            }
                            let order_output_ref = OutputRef::from(output.input);

                            let ve_factory_output_ref = pull_onchain_entity::<VEFactorySnapshot, _>(
                                &op_inputs.explorer,
                                protocol_deployment.ve_factory.hash,
                                op_inputs.network_id,
                                &deployment_config,
                                VEFactoryId,
                            )
                            .await
                            .map(|(_, ve_factory_output)| OutputRef::from(ve_factory_output.input))
                            .unwrap();
                            let mut values = [
                                (T::Order, order_output_ref),
                                (T::VE, ve_output_ref),
                                (T::VEFactory, ve_factory_output_ref),
                            ];
                            values.sort_by(|(_, x), (_, y)| x.cmp(y));

                            let order_input_ix =
                                values.iter().position(|(t, _)| matches!(t, T::Order)).unwrap() as u32;
                            let voting_escrow_input_ix =
                                values.iter().position(|(t, _)| matches!(t, T::VE)).unwrap() as u32;
                            let ve_factory_input_ix = values
                                .iter()
                                .position(|(t, _)| matches!(t, T::VEFactory))
                                .unwrap() as u32;

                            let order_action = ExtendVotingEscrowOrderAction::Extend {
                                order_input_ix,
                                voting_escrow_input_ix,
                                ve_factory_input_ix,
                            };
                            let id = OffChainOrderId {
                                voting_escrow_id,
                                version,
                            };
                            let offchain_order = create_extend_ve_offchain_order(
                                id,
                                order_action,
                                order_output_ref,
                                &deployment_config,
                                &op_inputs.operator_sk,
                            );
                            send_extend_ve_offchain_order(
                                offchain_order,
                                &op_inputs.voting_order_listener_endpoint,
                            )
                            .await;

                            ve_state = VEState::PredictedOffChainExtendedVESent(e, VEVersion(version));
                        }
                    }
                    VEState::PredictedOffChainExtendedVESent(epoch, VEVersion(version)) => {
                        if let Some((ve_snapshot, _)) = pull_onchain_entity::<VotingEscrowSnapshot, _>(
                            &op_inputs.explorer,
                            protocol_deployment.voting_escrow.hash,
                            op_inputs.network_id,
                            &deployment_config,
                            voting_escrow_id,
                        )
                        .await
                        {
                            if ve_snapshot.get().version > version as u32 {
                                ve_state = VEState::ConfirmedVoteCast {
                                    ve_snapshot,
                                    ve_extended_this_epoch: true,
                                };
                            }
                        }
                    }
                    VEState::ConfirmedVotingEscrow(ref ve_snapshot, Epoch(epoch)) => {
                        // cast vote
                        let voting_power = ve_snapshot.get().voting_power(now);
                        let version = ve_snapshot.get().version as u64;
                        println!(
                            "Voting from state {:?}, epoch: {}, voting_power: {}, version: {}",
                            ve_state, epoch, voting_power, version
                        );
                        let wpoll_policy_id = deployment_config.deployed_validators.mint_wpauth_token.hash;
                        let voting_order_id = OffChainOrderId {
                            voting_escrow_id,
                            version,
                        };
                        let voting_order = create_voting_order(
                            &op_inputs.operator_sk,
                            voting_order_id,
                            voting_power,
                            wpoll_policy_id,
                            epoch,
                            op_inputs.dao_parameters.num_active_farms,
                        );

                        send_vote(voting_order, &op_inputs.voting_order_listener_endpoint).await;

                        ve_state = VEState::PredictedVoteCast(Epoch(current_epoch));
                    }
                    VEState::Waiting(owner) => {
                        let voting_escrow_id =
                            VotingEscrowId(spectrum_cardano_lib::AssetName::from(ve_id.clone()));

                        println!("VEState::Waiting: start");
                        if let Some((ve_snapshot, _)) = pull_onchain_entity::<VotingEscrowSnapshot, _>(
                            &op_inputs.explorer,
                            protocol_deployment.voting_escrow.hash,
                            op_inputs.network_id,
                            &deployment_config,
                            voting_escrow_id,
                        )
                        .await
                        {
                            println!("VEState::Waiting: found VE");
                            let can_redeem = match ve_snapshot.get().locked_until {
                                Lock::Def(until) => {
                                    let now = SystemTime::now()
                                        .duration_since(SystemTime::UNIX_EPOCH)
                                        .unwrap()
                                        .as_millis() as u64;
                                    now > until
                                }
                                Lock::Indef(duration) => todo!(),
                            };
                            if can_redeem {
                                println!("Redeem VE---------------------------");
                                let ve_factory_output_ref = pull_onchain_entity::<VEFactorySnapshot, _>(
                                    &op_inputs.explorer,
                                    protocol_deployment.ve_factory.hash,
                                    op_inputs.network_id,
                                    &deployment_config,
                                    VEFactoryId,
                                )
                                .await
                                .map(|(_, ve_factory_output)| OutputRef::from(ve_factory_output.input))
                                .unwrap();

                                let ve_output_ref = ve_snapshot.version().output_ref;
                                let (voting_escrow_input_ix, ve_factory_input_ix) =
                                    if ve_output_ref < ve_factory_output_ref {
                                        (0, 1)
                                    } else {
                                        (1, 0)
                                    };
                                let id = OffChainOrderId {
                                    voting_escrow_id,
                                    version: ve_snapshot.get().version as u64,
                                };
                                let order = create_redeem_ve_offchain_order(
                                    id,
                                    voting_escrow_input_ix,
                                    ve_factory_input_ix,
                                    &deployment_config,
                                    &op_inputs.operator_sk,
                                    op_inputs.stake_credential.clone(),
                                );
                                send_redeem_ve_offchain_order(
                                    order,
                                    &op_inputs.voting_order_listener_endpoint,
                                )
                                .await;
                            }
                            //let eve_order_exists_onchain =
                            //    pull_onchain_entity::<ExtendVotingEscrowOnchainOrder, _>(
                            //        &op_inputs.explorer,
                            //        protocol_deployment.extend_ve_order.hash,
                            //        op_inputs.network_id,
                            //        &deployment_config,
                            //        owner,
                            //    )
                            //    .await
                            //    .is_some();
                            //if eve_order_exists_onchain {
                            //    println!("VEState::Waiting: EVE order exists on-chain");
                            //    ve_state = VEState::PredictedOnChainExtendedVE(
                            //        Epoch(current_epoch),
                            //        owner,
                            //        ve_snapshot.version().output_ref,
                            //        VEVersion(ve_snapshot.get().version as u64),
                            //    );
                            //} else {
                            //    println!("VEState::Waiting: no EVE order exists");
                            //    ve_state = VEState::ConfirmedVotingEscrow(ve_snapshot, Epoch(current_epoch));
                            //}
                        }
                    }
                    VEState::PredictedRedeem => {
                        panic!("Predicted redeem");
                    }
                }
            }
        }

        const DEFAULT_DELAY_MILLIS: u64 = 20_000;
        Delay::new(Duration::from_millis(DEFAULT_DELAY_MILLIS)).await;
    }
}

fn create_extend_ve_offchain_order(
    id: OffChainOrderId,
    order_action: ExtendVotingEscrowOrderAction,
    order_output_ref: OutputRef,
    config: &CompleteDeployment,
    operator_sk: &PrivateKey,
) -> ExtendVotingEscrowOffChainOrder {
    use cml_chain::Serialize;
    let witness: PlutusScript = compute_extend_ve_witness_validator().into();
    let ve_ident = (
        config.deployed_validators.mint_identifier.hash,
        cml_chain::assets::AssetName::from(id.voting_escrow_id.0),
    );
    let ve_factory = (
        config.minted_deployment_tokens.ve_factory_auth.policy_id,
        config.minted_deployment_tokens.ve_factory_auth.asset_name.clone(),
    );
    let redeemer = make_extend_ve_witness_redeemer(order_output_ref, order_action, ve_ident, ve_factory);
    let redeemer_hex = hex::encode(redeemer.to_cbor_bytes());
    println!("redeemer: {}", redeemer_hex);
    let message =
        compute_voting_escrow_witness_message(witness.hash(), redeemer_hex.clone(), id.version).unwrap();
    println!("message: {}", hex::encode(&message));
    let signature = operator_sk.sign(&message).to_raw_bytes().to_vec();
    ExtendVotingEscrowOffChainOrder {
        id,
        proof: signature,
        witness: witness.hash(),
        witness_input: redeemer_hex,
        order_output_ref,
    }
}

fn create_redeem_ve_offchain_order(
    id: OffChainOrderId,
    voting_escrow_input_ix: u32,
    ve_factory_input_ix: u32,
    config: &CompleteDeployment,
    operator_sk: &PrivateKey,
    stake_credential: StakeCredential,
) -> RedeemVotingEscrowOffChainOrder {
    use cml_chain::Serialize;
    let witness: PlutusScript = PlutusV3Script::new(
        hex::decode(&DaoScriptData::global().redeem_voting_escrow_witness.script_bytes).unwrap(),
    )
    .into();
    let ve_ident = (
        config.deployed_validators.mint_identifier.hash,
        cml_chain::assets::AssetName::from(id.voting_escrow_id.0),
    );
    let ve_factory = (
        config.minted_deployment_tokens.ve_factory_auth.policy_id,
        config.minted_deployment_tokens.ve_factory_auth.asset_name.clone(),
    );
    let witness_redeemer = make_redeem_ve_witness_redeemer(
        Some(stake_credential.clone()),
        voting_escrow_input_ix,
        ve_factory_input_ix,
        ve_ident,
        ve_factory,
        //    config.splash_tokens.policy_id,
        PolicyId::from_hex("7876492e3b82a31b1ce97a8f454cec653a0f6be5c09b90e62d24c152").unwrap(),
        config.deployed_validators.mint_ve_composition_token.hash,
    );
    let redeemer_hex = hex::encode(witness_redeemer.to_cbor_bytes());
    println!("redeemer: {}", redeemer_hex);
    let message =
        compute_voting_escrow_witness_message(witness.hash(), redeemer_hex.clone(), id.version).unwrap();
    println!("message: {}", hex::encode(&message));
    let signature = operator_sk.sign(&message).to_raw_bytes().to_vec();
    RedeemVotingEscrowOffChainOrder {
        id,
        stake_credential: Some(stake_credential),
        proof: signature,
        witness: witness.hash(),
        witness_input: redeemer_hex,
    }
}

async fn send_vote(voting_order: VotingOrder, voting_order_listener_endpoint: &SocketAddr) {
    let client = reqwest::Client::new();

    // Send the PUT request with JSON body
    let response = retry!(
        {
            let url = format!(
                "http://{}{}",
                &voting_order_listener_endpoint, "/submit/votingorder"
            );
            client
                .put(url)
                .json(&voting_order) // Serialize the payload as JSON
                .send()
                .await
        },
        100,
        2000
    );

    if let Ok(response) = response {
        if response.status().is_success() {
            let text = response.text().await.unwrap();
            println!("Vote response: {}", text);
        } else {
            println!("Failed with status: {}", response.status());
            let error_text = response.text().await.unwrap();
            println!("Error: {}", error_text);
        }
    }
}

async fn send_extend_ve_offchain_order(
    order: ExtendVotingEscrowOffChainOrder,
    voting_order_listener_endpoint: &SocketAddr,
) {
    let client = reqwest::Client::new();

    // Send the PUT request with JSON body
    let response = retry!(
        {
            let url = format!("http://{}{}", &voting_order_listener_endpoint, "/submit/extendve");
            client
                .put(url)
                .json(&order) // Serialize the payload as JSON
                .send()
                .await
        },
        100,
        2000
    );

    if let Ok(response) = response {
        if response.status().is_success() {
            let text = response.text().await.unwrap();
            println!("Vote response: {}", text);
        } else {
            println!("Failed with status: {}", response.status());
            let error_text = response.text().await.unwrap();
            println!("Error: {}", error_text);
        }
    }
}

async fn send_redeem_ve_offchain_order(
    order: RedeemVotingEscrowOffChainOrder,
    voting_order_listener_endpoint: &SocketAddr,
) {
    let client = reqwest::Client::new();

    // Send the PUT request with JSON body
    let response = retry!(
        {
            let url = format!("http://{}{}", &voting_order_listener_endpoint, "/submit/redeemve");
            client
                .put(url)
                .json(&order) // Serialize the payload as JSON
                .send()
                .await
        },
        100,
        2000
    );

    if let Ok(response) = response {
        if response.status().is_success() {
            let text = response.text().await.unwrap();
            println!("Vote response: {}", text);
        } else {
            println!("Failed with status: {}", response.status());
            let error_text = response.text().await.unwrap();
            println!("Error: {}", error_text);
        }
    }
}

async fn request_mve_status(
    owner: Owner,
    voting_order_listener_endpoint: &SocketAddr,
) -> Option<DaoOrderStatus> {
    let client = reqwest::Client::new();

    // Send the PUT request with JSON body
    let response = retry!(
        {
            let url = format!(
                "http://{}{}",
                &voting_order_listener_endpoint, "/query/ve/identifier/name"
            );
            client
                .put(url)
                .json(&owner) // Serialize the payload as JSON
                .send()
                .await
        },
        100,
        2000
    );
    let res = response.unwrap().json::<Option<DaoOrderStatus>>().await;

    println!("{:?}", res);
    res.unwrap()
}

#[derive(Clone, Debug)]
enum VEState {
    Waiting(Owner),
    ConfirmedVotingEscrow(VotingEscrowSnapshot, Epoch),
    ConfirmedVoteCast {
        ve_snapshot: VotingEscrowSnapshot,
        ve_extended_this_epoch: bool,
    },
    PredictedOnChainExtendedVE(Epoch, Owner, OutputRef, VEVersion),
    PredictedOffChainExtendedVESent(Epoch, VEVersion),
    PredictedVoteCast(Epoch),
    PredictedRedeem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Epoch(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VEVersion(u64);

#[derive(Deserialize, Serialize)]
struct UserVEIdentifier {
    identifier_name: Option<cml_chain::assets::AssetName>,
}
