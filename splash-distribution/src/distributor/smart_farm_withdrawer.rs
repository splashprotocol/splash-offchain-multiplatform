use crate::buffered_wallet_holder::BufferedWalletsHolder;
use crate::context::DistributorCreds;
use crate::entities::events::smart_farms::withdraw::SmartFarmWithdraw;
use crate::entities::smart_farm::{DistributorSmartFarmSnapshot, SmartFarm, SmartFarmStatus};
use crate::index::status_events_index::StatusEntitiesIndex;
use crate::smart_farm_holder::SmartFarmHolder;
use async_stream::stream;
use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionUnspentOutput};
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::{Transaction, TransactionInput, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::RequiredSigners;
use futures::Stream;
use log::info;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_offchain::domain::{EntitySnapshot, Has};
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::prover::operator::OperatorProver;
use spectrum_offchain_cardano::tx_submission::TxSubmissionChannel;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::onchain::permission_manager::{PermManagerId, PermManagerSnapshot};
use splash_dao_offchain::entities::onchain::smart_farm;
use splash_dao_offchain::entities::onchain::smart_farm::{FarmId, SmartFarmSnapshot};
use splash_dao_offchain::protocol_config::FarmAuthPolicy;
use splash_dao_offchain::state_projection::{StateProjectionRead, StateProjectionRocksDB};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;

pub struct SmartFarmWithdrawer<SmartFarmsStorage, PeerManagerHolder>
{
    requests: Receiver<SmartFarmWithdraw>,
    buffered_wallets_holder: Arc<Mutex<BufferedWalletsHolder>>,
    smart_farms_storage: Arc<Mutex<SmartFarmsStorage>>,
    peer_manager_holder: PeerManagerHolder,
}

impl<'a, SmartFarmsStorage, PeerManagerHolder> SmartFarmWithdrawer<SmartFarmsStorage, PeerManagerHolder>
where
    SmartFarmsStorage: SmartFarmHolder + 'a,
{
    pub fn new(
        requests: Receiver<SmartFarmWithdraw>,
        buffered_wallets_holder: Arc<Mutex<BufferedWalletsHolder>>,
        smart_farms_storage: Arc<Mutex<SmartFarmsStorage>>,
        peer_manager_holder: PeerManagerHolder,
    ) -> Self {
        Self {
            requests,
            buffered_wallets_holder,
            smart_farms_storage,
            peer_manager_holder,
        }
    }

    pub fn withdrawer_stream<
        Ctx: Has<DeployedValidator<{ ProtocolValidator::SmartFarm as u8 }>>
            + Has<DistributorCreds>
            + Has<FarmAuthPolicy>,
    >(
        mut self,
        ctx: Ctx,
        prover: OperatorProver,
        net: TxSubmissionChannel<6, Transaction>,
    ) -> impl Stream<Item = ()>
    where
        PeerManagerHolder: StateProjectionRead<PermManagerSnapshot, TransactionOutput> + Send + Sync,
    {
        stream! {
            loop {
                let mut smart_farm_storage_guard = self.smart_farms_storage.lock().await;
                if let Some(next_request) = self.requests.recv().await {

                    let smart_farms_to_withdraw: Vec<Bundled<DistributorSmartFarmSnapshot, TransactionOutput>> = smart_farm_storage_guard.get_free_farms_by_value(
                        next_request.value
                    ).await;

                    if smart_farms_to_withdraw.len() == 0 {
                        info!("There is no free smart farms to withdraw")
                    } else {
                        let mut tx_builder = constant_tx_builder();

                        let smart_farms: Vec<DistributorSmartFarmSnapshot> = smart_farms_to_withdraw.clone().into_iter().map(|event| {
                            event.0.clone()
                        }).collect();

                        smart_farm_storage_guard.update_smart_farms_statuses(
                            smart_farms.clone(),
                            SmartFarmStatus::SFWithdrawInProgress
                        ).await;

                        let perm_manager_input_ix = 0;

                        if let Some(peer_manager) = self.peer_manager_holder.read(PermManagerId {}).await {

                            let peer_manager_input = peer_manager.erased();

                            let perm_manager_unspent_input = TransactionUnspentOutput::new(
                                TransactionInput::from(peer_manager_input.version().output_ref),
                                peer_manager_input.clone().1,
                            );
                            tx_builder.add_reference_input(perm_manager_unspent_input.clone());

                            let redeemer = smart_farm::Redeemer {
                                successor_out_ix: 1,
                                action: smart_farm::Action::DistributeRewards {
                                    perm_manager_input_ix,
                                },
                            }
                            .into_pd();

                            let smart_farm_script_hash = ctx.select::<FarmAuthPolicy>().0;
                            let smart_farm_script =
                                PartialPlutusWitness::new(PlutusScriptWitness::Ref(smart_farm_script_hash), redeemer);

                            let DistributorCreds(distributor_key, distributor_address) = ctx.select::<DistributorCreds>();

                            smart_farms_to_withdraw.into_iter().for_each(|Bundled(farm, farm_in)| {
                                let smart_farm_input = SingleInputBuilder::new(
                                    TransactionInput::new(farm.version().tx_hash(), farm.version().index()),
                                    farm_in.clone(),
                                )
                                .plutus_script_inline_datum(smart_farm_script.clone(), RequiredSigners::from(vec![distributor_key.clone()]))
                                .unwrap();
                                tx_builder.add_input(smart_farm_input).unwrap();
                            });


                            let signed_tx_builder =
                                tx_builder.build(ChangeSelectionAlgo::Default, &distributor_address).unwrap();

                            let tx = prover.prove(signed_tx_builder);

                            if let Ok(_) = net.clone().submit_tx(tx).await {
                                // todo: mark requests as processed
                                info!("withdrawer_stream")
                            };
                            ()
                        }
                    }
                }
            }
        }
    }
}
