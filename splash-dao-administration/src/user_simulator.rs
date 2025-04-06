use cardano_explorer::retry;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{PlutusData, PlutusScript, PlutusV2Script, PlutusV3Script};
use cml_chain::PolicyId;
use cml_crypto::PrivateKey;
use cml_crypto::RawBytesEncoding;
use futures_timer::Delay;
use rand::Rng;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::plutus_data::{DatumExtension, IntoPlutusData};
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{OutputRef, Token};
use splash_dao_offchain::entities::offchain::RedeemVotingEscrowOffChainOrder;
use splash_dao_offchain::entities::offchain::{compute_witness_message, OffChainOrderId};
use splash_dao_offchain::entities::offchain::{ExtendVotingEscrowOffChainOrder, WPollVoteOffChainOrder};
use splash_dao_offchain::entities::onchain::proxy_order_witness::{OwnerRedemptionUTxO, WitnessAction};
use splash_dao_offchain::entities::onchain::redeem_voting_escrow::{
    RedeemVEOrderAction, RedeemVotingEscrowOnchainOrder,
};
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::entities::onchain::voting_escrow::{Lock, VotingEscrowConfig};
use splash_dao_offchain::entities::onchain::voting_escrow_factory::VEFactoryId;
use splash_dao_offchain::entities::onchain::voting_escrow_factory::VEFactorySnapshot;
use splash_dao_offchain::entities::onchain::wpoll_vote_order::{WPollVoteAction, WPollVoteOnchainOrder};
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
    voting_order::create_offchain_voting_order, AppConfig, OperationInputs, VotingEscrowSettings,
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
                ve_state = VEState::ConfirmedVotingEscrow(ve_snapshot, ve_datum, Epoch(current_epoch));
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
                            let last_wp_epoch = ve_snapshot.get().last_wp_epoch;
                            if current_epoch as i32 == last_wp_epoch {
                                println!("Vote confirmed for epoch {}", current_epoch);
                                let ve_datum = VotingEscrowConfig::try_from_pd(
                                    unspent_output.output.datum().unwrap().into_pd().unwrap(),
                                )
                                .unwrap();
                                ve_state = VEState::ConfirmedVoteCast {
                                    ve_snapshot,
                                    ve_datum,
                                    ve_extended_this_epoch: false,
                                };
                            }
                        }
                    }
                    VEState::ConfirmedVoteCast {
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

                            create_wpoll_vote_onchain_order(
                                voting_escrow_id,
                                CurrentEpoch(current_epoch),
                                op_inputs,
                            )
                            .await;

                            ve_state = VEState::ConfirmedOnChainWPollVote(ve_snapshot.clone(), ve_datum);
                        } else if !ve_extended_this_epoch {
                            let owner = create_extend_voting_escrow_onchain_order(
                                voting_escrow_id,
                                &ve_settings,
                                op_inputs,
                            )
                            .await;
                            let ve_output_ref = *ve_snapshot.version();
                            ve_state = VEState::ConfirmedOnChainExtendedVE(ve_snapshot.clone(), ve_datum);
                        }
                    }
                    VEState::ConfirmedOnChainWPollVote(ref ve_snapshot, ve_datum) => {
                        //
                        // If order UTxO is found we can then send off-chain order to the bot.
                        let proxy_order_script_hash = protocol_deployment.wpoll_vote_order.hash;
                        if let Some(results) = pull_onchain_entity::<WPollVoteOnchainOrder, _>(
                            &op_inputs.explorer,
                            proxy_order_script_hash,
                            op_inputs.network_id,
                            &deployment_config,
                            owner,
                        )
                        .await
                        {
                            if let Some((_, order_output)) =
                                results.iter().find(|(order, _)| order.ve_datum == ve_datum)
                            {
                                enum T {
                                    Order,
                                    VE,
                                    WPoll,
                                }
                                let order_output_ref = OutputRef::from(order_output.input.clone());
                                let wpoll_output_ref = pull_onchain_entity::<WeightingPollSnapshot, _>(
                                    &op_inputs.explorer,
                                    protocol_deployment.mint_wpauth_token.hash,
                                    op_inputs.network_id,
                                    &deployment_config,
                                    WeightingPollId(current_epoch),
                                )
                                .await
                                .map(|mut result| {
                                    assert_eq!(result.len(), 1);
                                    let (_, output) = result.pop().unwrap();
                                    OutputRef::from(output.input)
                                })
                                .unwrap();
                                let mut typed_output_refs = [
                                    (T::Order, order_output_ref),
                                    (T::VE, *ve_snapshot.version()),
                                    (T::WPoll, wpoll_output_ref),
                                ];
                                typed_output_refs.sort_by(|(_, x), (_, y)| x.cmp(y));
                                let proxy_order_input_ix = typed_output_refs
                                    .iter()
                                    .position(|(t, _)| matches!(t, T::Order))
                                    .unwrap()
                                    as u32;
                                let voting_escrow_input_ix = typed_output_refs
                                    .iter()
                                    .position(|(t, _)| matches!(t, T::VE))
                                    .unwrap()
                                    as u32;
                                let wpoll_input_ix = typed_output_refs
                                    .iter()
                                    .position(|(t, _)| matches!(t, T::WPoll))
                                    .unwrap() as u32;

                                let ve_identifier_token = Token(
                                    deployment_config.deployed_validators.mint_identifier.hash,
                                    spectrum_cardano_lib::AssetName::from(ve_id),
                                );
                                let weighting_poll_auth_token = Token(
                                    deployment_config.deployed_validators.mint_wpauth_token.hash,
                                    spectrum_cardano_lib::AssetName::from(compute_epoch_asset_name(
                                        current_epoch,
                                    )),
                                );
                                let mut rng = rand::thread_rng();
                                let num_farms = op_inputs.dao_parameters.num_active_farms;
                                let chosen_id = rng.gen_range(0..num_farms);
                                let voting_power = ve_snapshot.get().voting_power(now);
                                let expected_diff: Vec<_> = (0..num_farms)
                                    .filter_map(|id| {
                                        if id == chosen_id {
                                            Some((
                                                FarmId(spectrum_cardano_lib::AssetName::from(
                                                    compute_farm_name(id),
                                                )),
                                                voting_power,
                                            ))
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();

                                let wpoll_vote_order_redeemer = WPollVoteAction::CastVote {
                                    weighting_poll_auth_token,
                                    ve_identifier_token,
                                    voting_escrow_input_ix,
                                    wpoll_input_ix,
                                    expected_diff: expected_diff.clone(),
                                };
                                let witness_action = WitnessAction {
                                    proxy_order_input_ix,
                                    proxy_order_output_reference: order_output_ref,
                                    proxy_order_script_hash,
                                    proxy_order_redeemer: wpoll_vote_order_redeemer.into_pd(),
                                    proxy_order_datum: ve_datum.into_pd(),
                                    owner_redemption: None,
                                };
                                let version = ve_snapshot.get().version;
                                let voting_order_id = OffChainOrderId {
                                    voting_escrow_id,
                                    version,
                                };

                                let offchain_order = create_offchain_voting_order(
                                    &op_inputs.operator_sk,
                                    expected_diff,
                                    voting_order_id,
                                    witness_action.into_pd(),
                                    order_output_ref,
                                );
                                println!(
                                    "voting_order JSON: {}",
                                    serde_json::to_string_pretty(&offchain_order).unwrap()
                                );

                                send_vote(offchain_order, &op_inputs.voting_order_listener_endpoint).await;
                                ve_state = VEState::PredictedVoteCast(Epoch(current_epoch));
                            }
                        }
                    }
                    VEState::ConfirmedOnChainExtendedVE(ref ve_snapshot, ve_datum) => {
                        // If order UTxO is found we can then send off-chain order to the bot.
                        let proxy_order_script_hash = protocol_deployment.extend_ve_order.hash;
                        if let Some(results) = pull_onchain_entity::<ExtendVotingEscrowOnchainOrder, _>(
                            &op_inputs.explorer,
                            proxy_order_script_hash,
                            op_inputs.network_id,
                            &deployment_config,
                            owner,
                        )
                        .await
                        {
                            println!("{} EVEs, against ve_datum: {:?}", results.len(), ve_datum);
                            if let Some((order, output)) = results.iter().find(|(order, _)| {
                                let VotingEscrowConfig {
                                    owner,
                                    version,
                                    last_wp_epoch,
                                    last_gp_deadline,
                                    ..
                                } = order.ve_datum;
                                owner == ve_datum.owner
                                    && version == ve_datum.version + 1
                                    && last_gp_deadline == ve_datum.last_gp_deadline
                                    && last_wp_epoch == ve_datum.last_wp_epoch
                            }) {
                                println!("FOUND!-----------------------------------------");
                                enum T {
                                    Order,
                                    VE,
                                    VEFactory,
                                }
                                let order_output_ref = OutputRef::from(output.input.clone());

                                let ve_factory_output_ref = pull_onchain_entity::<VEFactorySnapshot, _>(
                                    &op_inputs.explorer,
                                    protocol_deployment.ve_factory.hash,
                                    op_inputs.network_id,
                                    &deployment_config,
                                    VEFactoryId,
                                )
                                .await
                                .map(|mut result| {
                                    assert_eq!(result.len(), 1);
                                    let (_, ve_factory_output) = result.pop().unwrap();
                                    OutputRef::from(ve_factory_output.input)
                                })
                                .unwrap();
                                let mut values = [
                                    (T::Order, order_output_ref),
                                    (T::VE, *ve_snapshot.version()),
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
                                    .unwrap()
                                    as u32;

                                let order_action = ExtendVotingEscrowOrderAction::Extend {
                                    order_input_ix,
                                    voting_escrow_input_ix,
                                    ve_factory_input_ix,
                                };
                                let witness_action = WitnessAction {
                                    proxy_order_input_ix: order_input_ix,
                                    proxy_order_output_reference: order_output_ref,
                                    proxy_order_script_hash,
                                    proxy_order_redeemer: order_action.into_pd(),
                                    proxy_order_datum: order.ve_datum.into_pd(),
                                    owner_redemption: None,
                                };
                                let version = ve_snapshot.get().version;
                                let id = OffChainOrderId {
                                    voting_escrow_id,
                                    version,
                                };
                                let offchain_order = create_extend_ve_offchain_order(
                                    id,
                                    witness_action.into_pd(),
                                    order_output_ref,
                                    &op_inputs.operator_sk,
                                );
                                println!(
                                    "extend_ve_offchain_order: {}",
                                    serde_json::to_string_pretty(&offchain_order).unwrap()
                                );
                                send_extend_ve_offchain_order(
                                    offchain_order,
                                    &op_inputs.voting_order_listener_endpoint,
                                )
                                .await;

                                ve_state = VEState::PredictedOffChainExtendedVESent(VEVersion(version));
                            }
                        }
                    }
                    VEState::ConfirmedOnChainRedeemVE {
                        ve_version,
                        ve_output_ref,
                        ve_datum,
                        order_output_ref,
                    } => {
                        //
                        println!("Redeem VE---------------------------");
                        let ve_factory_output_ref = pull_onchain_entity::<VEFactorySnapshot, _>(
                            &op_inputs.explorer,
                            protocol_deployment.ve_factory.hash,
                            op_inputs.network_id,
                            &deployment_config,
                            VEFactoryId,
                        )
                        .await
                        .map(|mut result| {
                            assert_eq!(result.len(), 1);
                            let (_, ve_factory_output) = result.pop().unwrap();
                            OutputRef::from(ve_factory_output.input)
                        })
                        .unwrap();

                        enum T {
                            VE,
                            VEFACTORY,
                            ORDER,
                        }
                        let mut typed_output_refs = [
                            (T::VE, ve_output_ref),
                            (T::VEFACTORY, ve_factory_output_ref),
                            (T::ORDER, order_output_ref),
                        ];

                        typed_output_refs.sort_by(|(_, x), (_, y)| x.cmp(y));

                        let voting_escrow_input_ix = typed_output_refs
                            .iter()
                            .position(|(t, _)| matches!(t, T::VE))
                            .unwrap() as u32;
                        let ve_factory_input_ix = typed_output_refs
                            .iter()
                            .position(|(t, _)| matches!(t, T::VEFACTORY))
                            .unwrap() as u32;
                        let proxy_order_input_ix = typed_output_refs
                            .iter()
                            .position(|(t, _)| matches!(t, T::ORDER))
                            .unwrap() as u32;
                        let id = OffChainOrderId {
                            voting_escrow_id,
                            version: ve_version,
                        };

                        let owner_stake_credential = op_inputs.stake_credential.clone();
                        let proxy_order_redeemer = RedeemVEOrderAction::RedeemVE {
                            ve_identifier_token_name: voting_escrow_id.0,
                            owner_stake_credential: Some(owner_stake_credential.clone()),
                            voting_escrow_input_ix,
                            ve_factory_input_ix,
                        }
                        .into_pd();

                        let witness_redeemer = WitnessAction {
                            proxy_order_input_ix,
                            proxy_order_output_reference: order_output_ref,
                            proxy_order_script_hash: protocol_deployment.redeem_ve_order.hash,
                            proxy_order_redeemer,
                            proxy_order_datum: ve_datum.into_pd(),
                            owner_redemption: Some(OwnerRedemptionUTxO {
                                owner_output_ix: 1,
                                owner_stake_credential: owner_stake_credential.clone(),
                            }),
                        }
                        .into_pd();

                        let order = create_redeem_ve_offchain_order(
                            id,
                            witness_redeemer,
                            order_output_ref,
                            &op_inputs.operator_sk,
                            owner_stake_credential,
                        );
                        println!(
                            "redeem VE JSON: {}",
                            serde_json::to_string_pretty(&order).unwrap()
                        );
                        send_redeem_ve_offchain_order(order, &op_inputs.voting_order_listener_endpoint).await;
                    }
                    VEState::PredictedOffChainExtendedVESent(VEVersion(version)) => {
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
                            if ve_snapshot.get().version > version {
                                ve_state = VEState::ConfirmedVoteCast {
                                    ve_snapshot,
                                    ve_datum,
                                    ve_extended_this_epoch: true,
                                };
                            }
                        }
                    }
                    VEState::ConfirmedVotingEscrow(ref ve_snapshot, ve_datum, Epoch(epoch)) => {
                        create_wpoll_vote_onchain_order(
                            voting_escrow_id,
                            CurrentEpoch(current_epoch),
                            op_inputs,
                        )
                        .await;

                        ve_state = VEState::ConfirmedOnChainWPollVote(ve_snapshot.clone(), ve_datum);
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
                                            if order.ve_datum == ve_datum {
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
                                    create_redeem_voting_escrow_onchain_order(
                                        unspent_output.output.datum().unwrap(),
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
                                        } = order.ve_datum;
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
                                    if ve_datum.last_wp_epoch == current_epoch as i32 {
                                        ve_state = VEState::ConfirmedVoteCast {
                                            ve_snapshot,
                                            ve_datum,
                                            ve_extended_this_epoch: false,
                                        };
                                    } else {
                                        println!("Haven't cast vote in epoch {}", current_epoch);
                                        ve_state = VEState::ConfirmedVotingEscrow(
                                            ve_snapshot,
                                            ve_datum,
                                            Epoch(current_epoch),
                                        );
                                    }
                                }
                            }
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
    witness_redeemer: PlutusData,
    order_output_ref: OutputRef,
    operator_sk: &PrivateKey,
) -> ExtendVotingEscrowOffChainOrder {
    use cml_chain::Serialize;
    let witness = PlutusScript::PlutusV3(PlutusV3Script::new(
        hex::decode(&DaoScriptData::global().proxy_order_witness.script_bytes).unwrap(),
    ));
    let redeemer_hex = hex::encode(witness_redeemer.to_cbor_bytes());
    println!("redeemer: {}", redeemer_hex);
    let message = compute_witness_message(witness.hash(), redeemer_hex.clone(), id.version as u64).unwrap();
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
    witness_redeemer: PlutusData,
    order_output_ref: OutputRef,
    operator_sk: &PrivateKey,
    stake_credential: StakeCredential,
) -> RedeemVotingEscrowOffChainOrder {
    use cml_chain::Serialize;
    let witness: PlutusScript =
        PlutusV3Script::new(hex::decode(&DaoScriptData::global().proxy_order_witness.script_bytes).unwrap())
            .into();
    let redeemer_hex = hex::encode(witness_redeemer.to_cbor_bytes());
    println!("redeemer: {}", redeemer_hex);
    let message = compute_witness_message(witness.hash(), redeemer_hex.clone(), id.version as u64).unwrap();
    println!("message: {}", hex::encode(&message));
    let signature = operator_sk.sign(&message).to_raw_bytes().to_vec();
    RedeemVotingEscrowOffChainOrder {
        id,
        stake_credential: Some(stake_credential),
        proof: signature,
        witness: witness.hash(),
        witness_input: redeemer_hex,
        order_output_ref,
    }
}

async fn send_vote(voting_order: WPollVoteOffChainOrder, voting_order_listener_endpoint: &SocketAddr) {
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
                .ok()
        },
        100,
        2000
    );

    if let Some(response) = response {
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
                .ok()
        },
        100,
        2000
    );

    if let Some(response) = response {
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
                .ok()
        },
        100,
        2000
    );

    if let Some(response) = response {
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
    ConfirmedVotingEscrow(VotingEscrowSnapshot, VotingEscrowConfig, Epoch),
    ConfirmedVoteCast {
        ve_snapshot: VotingEscrowSnapshot,
        ve_datum: VotingEscrowConfig,
        ve_extended_this_epoch: bool,
    },
    ConfirmedOnChainWPollVote(VotingEscrowSnapshot, VotingEscrowConfig),
    ConfirmedOnChainExtendedVE(VotingEscrowSnapshot, VotingEscrowConfig),
    ConfirmedOnChainRedeemVE {
        ve_version: u32,
        ve_output_ref: OutputRef,
        ve_datum: VotingEscrowConfig,
        order_output_ref: OutputRef,
    },
    PredictedOffChainExtendedVESent(VEVersion),
    PredictedVoteCast(Epoch),
    PredictedRedeem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Epoch(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VEVersion(u32);

#[derive(Deserialize, Serialize)]
struct UserVEIdentifier {
    identifier_name: Option<cml_chain::assets::AssetName>,
}
