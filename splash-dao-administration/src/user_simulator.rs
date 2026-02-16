use cardano_explorer::retry;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{PlutusData, PlutusScript, PlutusV2Script, PlutusV3Script};
use cml_chain::PolicyId;
use cml_crypto::{PrivateKey, ScriptHash};
use cml_crypto::{RawBytesEncoding, TransactionHash};
use futures_timer::Delay;
use rand::Rng;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::plutus_data::{DatumExtension, IntoPlutusData};
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{OutputRef, Token};
use splash_dao_offchain::entities::offchain::OffChainOrderId;
use splash_dao_offchain::entities::offchain::{compute_witness_message, RedeemVotingEscrowOffChainOrder};
use splash_dao_offchain::entities::offchain::{ExtendVotingEscrowOffChainOrder, WPollVoteOffChainOrder};
use splash_dao_offchain::entities::onchain::proxy_order_witness::{OwnerRedemptionUTxO, WitnessAction};
use splash_dao_offchain::entities::onchain::redeem_voting_escrow::{
    RedeemVEOrderAction, RedeemVotingEscrowOnchainOrder, RedeemVotingEscrowOrderState,
};
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::entities::onchain::voting_escrow::{Lock, VotingEscrowConfig};
use splash_dao_offchain::entities::onchain::voting_escrow_factory::VEFactoryId;
use splash_dao_offchain::entities::onchain::voting_escrow_factory::VEFactorySnapshot;
use splash_dao_offchain::entities::onchain::wpoll_vote_order::{
    WPollVoteAction, WPollVoteOnchainOrder, WPollVoteState,
};
use splash_dao_offchain::entities::onchain::ProxyOrderMetadata;
use splash_dao_offchain::routines::actions::{compute_epoch_asset_name, compute_farm_name};
use splash_dao_offchain::{
    deployment::{CompleteDeployment, DaoScriptData, ProtocolDeployment},
    entities::onchain::{
        extend_voting_escrow_order::{ExtendVotingEscrowOnchainOrder, ExtendVotingEscrowOrderAction},
        make_voting_escrow_order::DaoOrderStatus,
        voting_escrow::{Owner, VotingEscrowId, VotingEscrowSnapshot},
        weighting_poll::{WeightingPollId, WeightingPollSnapshot},
    },
    routines::time_millis_to_epoch,
    time::epoch_start,
    CurrentEpoch, GenesisEpochStartTime,
};
use std::{
    net::SocketAddr,
    time::{Duration, SystemTime},
};
use tokio::io::AsyncWriteExt;

use crate::{
    create_extend_voting_escrow_onchain_order, deploy, make_voting_escrow_order, pull_onchain_entity,
    AppConfig, OperationInputs, VotingEscrowSettings,
};
use crate::{create_redeem_voting_escrow_onchain_order, create_wpoll_vote_onchain_order};

const EPOCH_WAIT_TIME: u64 = 30_000;

pub async fn user_simulator<'a>(
    op_inputs: &mut OperationInputs,
    config: AppConfig<'a>,
    ve_identifier_json_path: &str,
    assets_json_path: &str,
    existing_splash_policy_id: Option<PolicyId>,
) -> ! {
    // 1. deploy
    let deployment_config = deploy(op_inputs, config, existing_splash_policy_id).await;

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
            if let Some(mut results) = pull_onchain_entity::<VotingEscrowSnapshot, _>(
                &op_inputs.explorer,
                protocol_deployment.voting_escrow.hash,
                op_inputs.network_id,
                &deployment_config,
                voting_escrow_id,
            )
            .await
            {
                assert_eq!(results.len(), 1);
                let (ve_snapshot, unspent_output) = results.pop().unwrap();
                let ve_datum = VotingEscrowConfig::try_from_pd(
                    unspent_output.output.datum().unwrap().into_pd().unwrap(),
                )
                .unwrap();
                ve_state = VEState::WithVotingEscrow {
                    ve_snapshot,
                    ve_datum,
                    ve_extended_this_epoch: false, // set this to true to prevent lock extension to next epoch
                };
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
                let ve_identifier_token_name = spectrum_cardano_lib::AssetName::from(ve_id.clone());
                let weighting_poll_auth_token_name =
                    spectrum_cardano_lib::AssetName::from(compute_epoch_asset_name(current_epoch));
                match ve_state {
                    VEState::WithVotingEscrow {
                        ref ve_snapshot,
                        ve_datum,
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

                            let available_voting_power = ve_snapshot.get().voting_power(now);
                            assert!(available_voting_power > 200);
                            let voting_power = available_voting_power - 200;
                            let num_farms = op_inputs.deployment_progress.initial_farms.len() as u32;
                            let expected_diff: Vec<_> = (0..num_farms)
                                .map(|id| {
                                    (
                                        FarmId(spectrum_cardano_lib::AssetName::from(compute_farm_name(id))),
                                        voting_power / num_farms as u64,
                                    )
                                })
                                .collect();
                            let proxy_order_script_hash = protocol_deployment.wpoll_vote_order.hash;
                            let onchain_order_output_ref = if let Some(results) =
                                pull_onchain_entity::<WPollVoteOnchainOrder, _>(
                                    &op_inputs.explorer,
                                    proxy_order_script_hash,
                                    op_inputs.network_id,
                                    &deployment_config,
                                    owner,
                                )
                                .await
                            {
                                if let Some((_, order_output)) =
                                    results.iter().find(|(order, _)| order.datum.ve_state == ve_datum)
                                {
                                    println!(
                                        "Waiting for bot to process wpoll {:?} vote for VE {}",
                                        order_output,
                                        hex::encode(ve_snapshot.get().ve_identifier_name.as_bytes())
                                    );
                                    order_output.clone()
                                } else {
                                    create_wpoll_vote_onchain_order(
                                        voting_escrow_id,
                                        ve_identifier_token_name,
                                        weighting_poll_auth_token_name,
                                        expected_diff.clone(),
                                        op_inputs,
                                    )
                                    .await
                                    .unwrap()
                                }
                            } else {
                                create_wpoll_vote_onchain_order(
                                    voting_escrow_id,
                                    ve_identifier_token_name,
                                    weighting_poll_auth_token_name,
                                    expected_diff.clone(),
                                    op_inputs,
                                )
                                .await
                                .unwrap()
                            };

                            let order_datum = WPollVoteState {
                                ve_state: ve_datum,
                                weighting_poll_auth_token_name,
                                ve_identifier_token_name,
                                expected_diff,
                            };

                            println!("Confirmed on chain WPoll vote");
                            ve_state = VEState::ConfirmedOnChainWPollVote {
                                ve_snapshot: ve_snapshot.clone(),
                                order_datum,
                                onchain_order_output_ref,
                            };
                        } else if !ve_extended_this_epoch {
                            println!("Extending VE");
                            let owner = create_extend_voting_escrow_onchain_order(
                                voting_escrow_id,
                                ve_identifier_token_name,
                                &ve_settings,
                                op_inputs,
                            )
                            .await;
                            let ve_output_ref = *ve_snapshot.version();
                            ve_state = VEState::ConfirmedOnChainExtendedVE(ve_snapshot.clone(), ve_datum);
                        }
                    }
                    VEState::ConfirmedOnChainWPollVote {
                        ref ve_snapshot,
                        ref order_datum,
                        ..
                    } => {
                        // Check if bot has processed it!!!!!!!
                        let proxy_order_script_hash = protocol_deployment.wpoll_vote_order.hash;
                        if pull_onchain_entity::<WPollVoteOnchainOrder, _>(
                            &op_inputs.explorer,
                            proxy_order_script_hash,
                            op_inputs.network_id,
                            &deployment_config,
                            owner,
                        )
                        .await
                        .is_none()
                        {
                            println!(
                                "VE {}: WPoll vote order processed by bot",
                                hex::encode(ve_snapshot.get().ve_identifier_name.as_bytes())
                            );

                            ve_state = VEState::WithVotingEscrow {
                                ve_snapshot: ve_snapshot.clone(),
                                ve_datum: order_datum.ve_state,
                                ve_extended_this_epoch: false, // user always votes before extending VE
                            };
                        } else {
                            println!("WPoll vote order already processed by bot");
                        }
                    }
                    VEState::ConfirmedOnChainExtendedVE(ref ve_snapshot, ve_datum) => {
                        // If order UTxO is found we can then send off-chain order to the bot.
                        let proxy_order_script_hash = protocol_deployment.extend_ve_order.hash;
                        if pull_onchain_entity::<ExtendVotingEscrowOnchainOrder, _>(
                            &op_inputs.explorer,
                            proxy_order_script_hash,
                            op_inputs.network_id,
                            &deployment_config,
                            owner,
                        )
                        .await
                        .is_none()
                        {
                            println!(
                                "VE {} extended by bot",
                                hex::encode(ve_snapshot.get().ve_identifier_name.as_bytes())
                            );
                            ve_state = VEState::WithVotingEscrow {
                                ve_snapshot: ve_snapshot.clone(),
                                ve_datum,
                                ve_extended_this_epoch: true,
                            };
                        }
                    }
                    VEState::ConfirmedOnChainRedeemVE {
                        ve_version,
                        ve_output_ref,
                        ve_datum,
                        order_output_ref,
                    } => {
                        println!(
                            "Redeemed VE. Identifier: {}",
                            hex::encode(ve_identifier_token_name.as_bytes())
                        );
                    }
                    VEState::Waiting(owner) => {
                        let voting_escrow_id =
                            VotingEscrowId(spectrum_cardano_lib::AssetName::from(ve_id.clone()));

                        println!("VEState::Waiting: start");
                        if let Some(mut results) = pull_onchain_entity::<VotingEscrowSnapshot, _>(
                            &op_inputs.explorer,
                            protocol_deployment.voting_escrow.hash,
                            op_inputs.network_id,
                            &deployment_config,
                            voting_escrow_id,
                        )
                        .await
                        {
                            assert_eq!(results.len(), 1);
                            let (ve_snapshot, unspent_output) = results.pop().unwrap();
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
                            let ve_datum = VotingEscrowConfig::try_from_pd(
                                unspent_output.output.datum().unwrap().into_pd().unwrap(),
                            )
                            .unwrap();
                            if can_redeem {
                                println!("CAN REDEEM VE");
                                let order_output_ref = if let Some(Some(order_output_ref)) =
                                    pull_onchain_entity::<RedeemVotingEscrowOnchainOrder, _>(
                                        &op_inputs.explorer,
                                        protocol_deployment.extend_ve_order.hash,
                                        op_inputs.network_id,
                                        &deployment_config,
                                        owner,
                                    )
                                    .await
                                    .map(|order_results| {
                                        order_results.iter().find_map(|(order, unspent_output)| {
                                            if order.datum.ve_state == ve_datum {
                                                Some(OutputRef::from(unspent_output.input.clone()))
                                            } else {
                                                None
                                            }
                                        })
                                    }) {
                                    println!("Found existing redeem proxy order");
                                    order_output_ref
                                } else {
                                    println!("Creating new redeem proxy order");
                                    let owner_stake_credential = Some(op_inputs.stake_credential.clone());
                                    let order_datum = RedeemVotingEscrowOrderState {
                                        ve_state: ve_datum,
                                        ve_identifier_token_name,
                                        owner_stake_credential,
                                    };
                                    let order_metadata = create_ve_metadata(
                                        order_datum.clone(),
                                        protocol_deployment.redeem_ve_order.hash,
                                        order_datum.ve_state.version,
                                        &op_inputs.operator_sk,
                                    );
                                    create_redeem_voting_escrow_onchain_order(
                                        order_datum,
                                        order_metadata,
                                        op_inputs,
                                    )
                                    .await
                                };
                                ve_state = VEState::ConfirmedOnChainRedeemVE {
                                    ve_version: ve_snapshot.get().version,
                                    ve_output_ref: *ve_snapshot.version(),
                                    ve_datum,
                                    order_output_ref,
                                };
                            } else {
                                println!("CANNOT REDEEM VE");
                                let eve_order_exists_onchain = if let Some(results) =
                                    pull_onchain_entity::<ExtendVotingEscrowOnchainOrder, _>(
                                        &op_inputs.explorer,
                                        protocol_deployment.extend_ve_order.hash,
                                        op_inputs.network_id,
                                        &deployment_config,
                                        owner,
                                    )
                                    .await
                                {
                                    results.iter().any(|(order, _)| {
                                        let VotingEscrowConfig {
                                            owner,
                                            version,
                                            last_wp_epoch,
                                            last_gp_deadline,
                                            ..
                                        } = order.datum.ve_state;
                                        owner == ve_datum.owner
                                            && version == ve_datum.version + 1
                                            && last_gp_deadline == ve_datum.last_gp_deadline
                                            && last_wp_epoch == ve_datum.last_wp_epoch
                                    })
                                } else {
                                    false
                                };
                                let ve_datum = VotingEscrowConfig::try_from_pd(
                                    unspent_output.output.datum().unwrap().into_pd().unwrap(),
                                )
                                .unwrap();
                                if eve_order_exists_onchain {
                                    println!("VEState::Waiting: EVE order exists on-chain");
                                    ve_state = VEState::ConfirmedOnChainExtendedVE(ve_snapshot, ve_datum);
                                } else {
                                    println!("VEState::Waiting: no EVE order exists");
                                    ve_state = VEState::WithVotingEscrow {
                                        ve_snapshot,
                                        ve_datum,
                                        ve_extended_this_epoch: false,
                                    };
                                }
                            }
                        }
                    }
                }
            }
        }

        const DEFAULT_DELAY_MILLIS: u64 = 20_000;
        Delay::new(Duration::from_millis(DEFAULT_DELAY_MILLIS)).await;
    }
}

pub(crate) fn create_ve_metadata<T: IntoPlutusData + Clone>(
    order_datum: T,
    witness_script_hash: ScriptHash,
    version: u32,
    operator_sk: &PrivateKey,
) -> ProxyOrderMetadata {
    use cml_chain::Serialize;
    let datum_hex = hex::encode(order_datum.clone().into_pd().to_cbor_bytes());
    println!("datum: {}", datum_hex);
    let message = compute_witness_message(witness_script_hash, &order_datum.into_pd(), version);
    println!("message: {}", hex::encode(&message));
    let prefix_bytes = vec![0x9F, 1, 2, 3];
    let postfix_bytes = vec![0xFF];
    let mut full_payload: Vec<u8> = prefix_bytes.clone();
    full_payload.extend_from_slice(&message);
    full_payload.extend_from_slice(&postfix_bytes);
    let signature = operator_sk.sign(&full_payload).to_raw_bytes().to_vec();
    ProxyOrderMetadata {
        signature,
        witness_script_hash,
        prefix_bytes,
        postfix_bytes,
        version,
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
                .ok()
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
    WithVotingEscrow {
        ve_snapshot: VotingEscrowSnapshot,
        ve_datum: VotingEscrowConfig,
        ve_extended_this_epoch: bool,
    },
    /// UTxO for wpoll vote proxy-order was created.
    ConfirmedOnChainWPollVote {
        ve_snapshot: VotingEscrowSnapshot,
        order_datum: WPollVoteState,
        onchain_order_output_ref: TransactionUnspentOutput,
    },
    ConfirmedOnChainExtendedVE(VotingEscrowSnapshot, VotingEscrowConfig),
    ConfirmedOnChainRedeemVE {
        ve_version: u32,
        ve_output_ref: OutputRef,
        ve_datum: VotingEscrowConfig,
        order_output_ref: OutputRef,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Epoch(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VEVersion(u32);

#[derive(Deserialize, Serialize)]
struct UserVEIdentifier {
    identifier_name: Option<cml_chain::assets::AssetName>,
}
