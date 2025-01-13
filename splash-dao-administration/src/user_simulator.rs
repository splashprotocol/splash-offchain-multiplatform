use std::{
    net::SocketAddr,
    time::{Duration, SystemTime},
};

use cardano_explorer::retry;
use futures_timer::Delay;
use serde::{Deserialize, Serialize};
use splash_dao_offchain::{
    deployment::ProtocolDeployment,
    entities::{
        offchain::voting_order::{VotingOrder, VotingOrderId},
        onchain::{
            voting_escrow::{VotingEscrowId, VotingEscrowSnapshot},
            weighting_poll::{WeightingPollId, WeightingPollSnapshot},
        },
    },
    routines::inflation::time_millis_to_epoch,
    time::epoch_start,
    CurrentEpoch, GenesisEpochStartTime,
};
use tokio::io::AsyncWriteExt;

use crate::{
    deploy, make_voting_escrow, pull_onchain_entity, voting_order::create_voting_order, AppConfig,
    OperationInputs, VotingEscrowSettings,
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

    let protocol_deployment =
        ProtocolDeployment::unsafe_pull(deployment_config.deployed_validators.clone(), &op_inputs.explorer)
            .await;

    let s = std::fs::read_to_string(assets_json_path).expect("Cannot load voting_escrow settings JSON file");
    let ve_settings: VotingEscrowSettings =
        serde_json::from_str(&s).expect("Invalid voting_escrow settings file");

    // Current epoch as measured determined by weighting_poll
    let mut wpoll_current_epoch = None;
    let mut ve_state = VEState::Waiting;

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
            assert!(matches!(ve_state, VEState::Waiting));
            println!("---- Making deposit into VE");
            // Deposit into VE
            let identifier_name = make_voting_escrow(&ve_settings, op_inputs).await;

            let voting_escrow_id =
                VotingEscrowId(spectrum_cardano_lib::AssetName::from(identifier_name.clone()));
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
                        assert_eq!(e.0, current_epoch);
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
                                ve_state = VEState::ConfirmedVoteCast(ve_snapshot, e);
                            }
                        }
                    }
                    VEState::ConfirmedVoteCast(ref ve_snapshot, Epoch(epoch)) => {
                        // wait until epoch's ended (or add funds, vote on proposal)
                        let last_wp_epoch = ve_snapshot.get().last_wp_epoch;
                        assert_eq!(epoch as i32, last_wp_epoch);
                        if last_wp_epoch < current_epoch as i32 {
                            let version = ve_snapshot.get().version as u64;
                            println!(
                                "Voting from state {:?} in epoch {}, version: {}",
                                ve_state, current_epoch, version
                            );
                            let voting_power = ve_snapshot.get().voting_power(now);
                            let wpoll_policy_id =
                                deployment_config.deployed_validators.mint_wpauth_token.hash;
                            let voting_order_id = VotingOrderId {
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
                        }
                    }
                    VEState::ConfirmedVotingEscrow(ref ve_snapshot, Epoch(epoch)) => {
                        // cast vote
                        assert_eq!(epoch, current_epoch);
                        let voting_power = ve_snapshot.get().voting_power(now);
                        let version = ve_snapshot.get().version as u64;
                        println!(
                            "Voting from state {:?}, epoch: {}, voting_power: {}, version: {}",
                            ve_state, epoch, voting_power, version
                        );
                        let wpoll_policy_id = deployment_config.deployed_validators.mint_wpauth_token.hash;
                        let voting_order_id = VotingOrderId {
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
                    VEState::Waiting => {
                        let voting_escrow_id =
                            VotingEscrowId(spectrum_cardano_lib::AssetName::from(ve_id.clone()));

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
                    }
                }
            }
        }

        const DEFAULT_DELAY_MILLIS: u64 = 20_000;
        Delay::new(Duration::from_millis(DEFAULT_DELAY_MILLIS)).await;
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

#[derive(Clone, Debug)]
enum State {
    WaitingForWeightingPoll,
    NewWeightingPollFound(Epoch),
    PredictedNewVotingEscrow(Epoch),
    ConfirmedVotingEscrow(VotingEscrowSnapshot, Epoch),
    ConfirmedVoteCast(VotingEscrowSnapshot, Epoch),
    PredictedVoteCast(Epoch),
}

#[derive(Clone, Debug)]
enum WPollState {
    WaitingForWeightingPoll,
    NewWeightingPollFound(Epoch),
}

#[derive(Clone, Debug)]
enum VEState {
    Waiting,
    ConfirmedVotingEscrow(VotingEscrowSnapshot, Epoch),
    ConfirmedVoteCast(VotingEscrowSnapshot, Epoch),
    PredictedVoteCast(Epoch),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Epoch(u32);

#[derive(Deserialize, Serialize)]
struct UserVEIdentifier {
    identifier_name: Option<cml_chain::assets::AssetName>,
}
