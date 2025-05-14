use crate::buffered_wallet_holder::BufferedWalletsHolder;
use crate::context::DistributorCreds;
use crate::entities::events::smart_farms::withdraw::{SmartFarmWithdraw, SmartFarmWithdrawStatus};
use crate::entities::events::user::withdraw::{UserWithdraw, UserWithdrawStatus};
use crate::http_clients::lp_indexer_client::LpIndexerClient;
use crate::http_clients::validator_client::ValidatorClient;
use crate::index::status_events_index::StatusEntitiesIndex;
use async_stream::stream;
use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::assets::MultiAsset;
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::tx_builder::ChangeSelectionAlgo;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::{Transaction, TransactionInput};
use cml_chain::utils::BigInteger;
use cml_chain::{PolicyId, Value};
use futures::Stream;
use log::info;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, Token};
use spectrum_offchain::domain::Has;
use spectrum_offchain::network::Network;
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::tx_submission::TxSubmissionChannel;
use splash_dao_offchain::deployment::ProtocolValidator;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;
use splash_lp_indexer::onchain::event::MultipleAccountsHarvest;

pub struct UserRequestsProcessor<UW, Validator, IndexerClient>
where
    UW: StatusEntitiesIndex<OutputRef, MultipleAccountsHarvest>,
    Validator: ValidatorClient,
    IndexerClient: LpIndexerClient,
{
    user_withdraw_events: Arc<Mutex<UW>>,
    buffered_wallets_holder: Arc<Mutex<BufferedWalletsHolder>>,
    validator_client: Validator,
    lp_indexer_client: IndexerClient,
    sf_withdraw_requests_sender: Sender<SmartFarmWithdraw>,
}

// todo: fix me
pub fn splash_pair_id() -> Token {
    let policy_id = PolicyId::from_hex("splash_policy_id").unwrap();
    let token_name = AssetName::from_utf8("splash".to_string());
    Token(policy_id, token_name)
}

impl<'a, UW, Validator, IndexerClient> UserRequestsProcessor<UW, Validator, IndexerClient>
where
    UW: StatusEntitiesIndex<OutputRef, MultipleAccountsHarvest> + 'a,
    Validator: ValidatorClient + 'a,
    IndexerClient: LpIndexerClient + 'a,
{
    pub fn new(
        index: Arc<Mutex<UW>>,
        validator: Validator,
        buffered_wallets_holder: Arc<Mutex<BufferedWalletsHolder>>,
        indexer_client: IndexerClient,
        sf_withdraw_requests_sender: Sender<SmartFarmWithdraw>,
    ) -> Self {
        Self {
            user_withdraw_events: index,
            buffered_wallets_holder,
            validator_client: validator,
            lp_indexer_client: indexer_client,
            sf_withdraw_requests_sender,
        }
    }

    pub fn user_requests_processor_stream<
        Ctx: Has<DeployedValidator<{ ProtocolValidator::HarvestOrder as u8 }>> + Has<DistributorCreds> + 'a,
    >(
        self,
        ctx: Ctx,
        net: TxSubmissionChannel<6, Transaction>,
    ) -> impl Stream<Item = ()> + 'a {
        stream! {
            loop {
                let mut withdraw_events_guard = self.user_withdraw_events.lock().await;

                let mut events_to_process = withdraw_events_guard
                    .get_events_by_status(UserWithdrawStatus::New)
                    .await;

                // At first step we are mark events as in progress state
                events_to_process.clone().iter_mut().for_each(|event| {
                    withdraw_events_guard.update_event_status(event.1.1, UserWithdrawStatus::InProgress);
                });

                // At next step we should calculate how many we should withdraw from each token pair

                let mut requests_to_process_in_this_batch: Vec<(Bundled<UserWithdraw, FinalizedTxOut>, u64)> = Vec::new();
                let mut buffered_wallets_to_process_in_this_batch = Vec::new();
                let mut common_splash_value_to_withdraw = 0;
                let mut buffered_wallet_allocation = 0;

                for event in events_to_process.into_iter() {

                    let mut_user_requests = vec![];

                    for uset_account in event.clone().0.accounts {
                        if let Ok(Some(user_info)) = self.lp_indexer_client.get_user_info(uset_account.to_raw_bytes().to_hex()).await {
                        if common_splash_value_to_withdraw + user_info.user_available_splash >= buffered_wallet_allocation {
                            withdraw_events_guard.update_event_status(event.clone().1.clone().1.into(), UserWithdrawStatus::New).await;
                        } else {
                            let to_withdraw = common_splash_value_to_withdraw + user_info.user_available_splash - buffered_wallet_allocation;
                            let mut buffered_wallets_holder_guard = self.buffered_wallets_holder.lock().await;
                            let new_buffered_wallets =
                                buffered_wallets_holder_guard.get_wallets_and_reserve_for_withdraw(splash_pair_id(), to_withdraw, 0).await;
                            if new_buffered_wallets.len() == 0 {
                                let mut ma = MultiAsset::new();

                                ma.set(splash_pair_id().0, splash_pair_id().1.into(), to_withdraw);

                                let withdraw_request = SmartFarmWithdraw {
                                    status: SmartFarmWithdrawStatus::New,
                                    value: Value::new(0, ma)
                                };

                                withdraw_events_guard.update_event_status(event.clone().1.1.into(), UserWithdrawStatus::New).await;

                                let sending_result = self.sf_withdraw_requests_sender.send(withdraw_request).await;
                                info!("Send sf withdraw request. Result {}", sending_result.is_ok());
                            } else {
                                buffered_wallet_allocation += new_buffered_wallets.iter().map(|(wallet)| wallet.0.splash_amount).sum::<u64>();
                                buffered_wallets_to_process_in_this_batch.extend(new_buffered_wallets);
                                common_splash_value_to_withdraw += user_info.user_available_splash;
                                mut_user_requests.push((event, user_info.user_available_splash))
                            }
                        }
                    }
                    }
                };

                let mut tx_builder = constant_tx_builder();

                // Add buffered wallets

                for wallet_bundle in buffered_wallets_to_process_in_this_batch.into_iter() {
                    tx_builder.add_input(
                        SingleInputBuilder::new(
                            TransactionInput::new(
                                wallet_bundle.1.1.tx_hash(),
                                wallet_bundle.1.1.index()
                            ),
                            wallet_bundle.1.0
                        ).payment_key()
                        .unwrap()
                    ).unwrap()
                };

                // Add acceptable user requests

                let harvest_validator: DeployedValidator<{ ProtocolValidator::HarvestOrder as u8 }> = ctx.select::<DeployedValidator<{ ProtocolValidator::HarvestOrder as u8 }>>();

                // todo: move redeemer creation into separte module?
                let withdraw_witness =
                    PartialPlutusWitness::new(
                        PlutusScriptWitness::Ref(harvest_validator.hash),
                        PlutusData::new_integer(BigInteger::from(0))
                    );

                for (request, user_splash_to_withdraw) in requests_to_process_in_this_batch.iter() {
                    tx_builder.add_input(
                        SingleInputBuilder::new(
                            TransactionInput::new(
                                request.1.1.tx_hash(),
                                request.1.1.index()
                            ),
                            request.1.clone().0
                        ).plutus_script_inline_datum(
                            withdraw_witness.clone(),
                            vec![].into()
                        ).unwrap()
                    ).unwrap();

                    tx_builder.add_output(
                        SingleOutputBuilderResult::new(
                            request.0.finalize_withdraw(*user_splash_to_withdraw)
                        )
                    ).unwrap()
                };

                // Validator step

                let address: DistributorCreds = ctx.select::<DistributorCreds>();

                // todo: validate
                let signed_tx = tx_builder
                    .build(ChangeSelectionAlgo::Default, &address.1)
                    .unwrap();

                if let Some(tx) = self.validator_client.validate_tx(signed_tx).await {
                    if let Ok(_) = net.clone().submit_tx(tx).await {
                        info!("submit_tx");
                        requests_to_process_in_this_batch.iter().for_each(|(event, _)| {
                            withdraw_events_guard.update_event_status(event.1.1, UserWithdrawStatus::AwaitConfirmation);
                        })
                    }
                };
            }
        }
    }
}
