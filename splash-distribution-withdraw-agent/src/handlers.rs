use crate::context::entity_parsing_context::EntityParsingContext;
use crate::onchain::event::{BufferWalletAddress, OnChainEvent};
use async_trait::async_trait;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain_cardano::event_sink::tx_view::TxViewMut;
use cardano_chain_sync::data::LedgerTxEvent;
use cml_crypto::Ed25519KeyHash;
use log::info;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::onchain::permission_manager::PermManager;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::protocol_config::{FarmAuthPolicy, PermManagerAuthPolicy, SplashPolicy};
use splash_dao_offchain::routines::Slot;
use splash_distribution::entities::buffered_wallet::{BufferedWallet, BufferedWalletStatus};
use splash_distribution::entities::events::user::withdraw::{UserWithdraw, UserWithdrawStatus};
use splash_distribution::entities::perm_manager_entity::PermManagerEntity;
use splash_distribution::entities::smart_farm::SmartFarm;
use splash_distribution::index::status_events_index::StatusEntitiesIndex;
use splash_lp_indexer::config::HarvestLimits;
use splash_lp_indexer::onchain::event::MultipleAccountsHarvest;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct DAOEntitiesHandler<
    SmartFarmStorage,
    WithdrawRequestsStorage,
    BufferedWalletsStorage,
    PermManagerStorage,
    AppCtx,
> {
    smart_farms_storage: Arc<Mutex<SmartFarmStorage>>,
    withdraw_requests_storage: Arc<Mutex<WithdrawRequestsStorage>>,
    buffered_wallets_storage: Arc<Mutex<BufferedWalletsStorage>>,
    perm_manager_storage: Arc<Mutex<PermManagerStorage>>,
    ctx: AppCtx,
}

impl<SmartFarmStorage, WithdrawRequestsStorage, BufferedWalletsStorage, PermManagerStorage, AppCtx>
    DAOEntitiesHandler<
        SmartFarmStorage,
        WithdrawRequestsStorage,
        BufferedWalletsStorage,
        PermManagerStorage,
        AppCtx,
    >
{
    pub fn new(
        smart_farms_storage: Arc<Mutex<SmartFarmStorage>>,
        withdraw_requests_storage: Arc<Mutex<WithdrawRequestsStorage>>,
        buffered_wallets_storage: Arc<Mutex<BufferedWalletsStorage>>,
        perm_manager_storage: Arc<Mutex<PermManagerStorage>>,
        ctx: AppCtx,
    ) -> Self {
        Self {
            smart_farms_storage,
            withdraw_requests_storage,
            buffered_wallets_storage,
            perm_manager_storage,
            ctx,
        }
    }
}

#[async_trait]
impl<SmartFarmStorage, WithdrawRequestsStorage, BufferedWalletsStorage, PermManagerStorage, AppContext>
    EventHandler<LedgerTxEvent<TxViewMut>>
    for DAOEntitiesHandler<
        SmartFarmStorage,
        WithdrawRequestsStorage,
        BufferedWalletsStorage,
        PermManagerStorage,
        AppContext,
    >
where
    SmartFarmStorage: StatusEntitiesIndex<FarmId, SmartFarm> + Send,
    WithdrawRequestsStorage: StatusEntitiesIndex<OutputRef, UserWithdraw> + Send,
    BufferedWalletsStorage: StatusEntitiesIndex<OutputRef, BufferedWallet> + Send,
    PermManagerStorage: StatusEntitiesIndex<OutputRef, PermManagerEntity> + Send,
    AppContext: Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<SplashPolicy>
        + Has<HarvestLimits>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>
        + Has<BufferWalletAddress>
        + Clone
        + Send,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent<TxViewMut>) -> Option<LedgerTxEvent<TxViewMut>> {
        match ev.clone() {
            LedgerTxEvent::TxApplied {
                tx,
                slot,
                block_number,
                block_hash,
            } => {
                let mut smart_farm_storage_guard = self.smart_farms_storage.lock().await;
                let mut withdraw_requests_storage_guard = self.withdraw_requests_storage.lock().await;
                let mut buffered_wallet_storage_guard = self.buffered_wallets_storage.lock().await;
                let mut perm_manager_storage_guard = self.perm_manager_storage.lock().await;

                for input in tx.inputs {
                    // User withdraw check

                    if let Some(_) = withdraw_requests_storage_guard
                        .get_event_by_key(input.clone().into())
                        .await
                    {
                        info!(
                            "[DAO-Handler] Detected completed user withdraw in tx {}. Going to change status to Withdrawed",
                            tx.hash
                        );
                        withdraw_requests_storage_guard
                            .update_event_status(input.clone().into(), UserWithdrawStatus::Withdrawn)
                            .await
                    };

                    // Buffered wallet check

                    if let Some(_) = buffered_wallet_storage_guard
                        .get_event_by_key(input.clone().into())
                        .await
                    {
                        info!(
                            "[DAO-Handler] Detected spent buffered wallet in {}. Going to drop it from storage",
                            tx.hash
                        );
                        buffered_wallet_storage_guard
                            .drop_event(input.clone().into())
                            .await;
                    }
                }

                // Process outputs

                for (output_idx, output) in tx.outputs {
                    let entity_parsing_context = EntityParsingContext {
                        timed_output_ref: OutputRef::new(tx.hash, output_idx as u64),
                        transaction_signatures: tx.signers.clone(),
                        slot: Slot(slot),
                        app_context: self.ctx.clone(),
                    };
                    if let Some(event) = OnChainEvent::try_from_ledger(&output, &entity_parsing_context) {
                        match event {
                            OnChainEvent::MultipleHarvest(harvest_event) => {
                                info!("[DAO-Handler] Got harvest request {}", harvest_event);
                                withdraw_requests_storage_guard
                                    .put()
                            }
                            OnChainEvent::FarmStateUpdate(new_smart_farm) => {
                                if let Some(smart_farm_in_storage) =
                                    smart_farm_storage_guard.get_event_by_key(new_smart_farm.id).await
                                {
                                    info!("[DAO-Handler] Got already exists smart farm with id {} and status {}. Going to update it to new status {}", hex::encode(new_smart_farm.id.0.as_bytes()), smart_farm_in_storage.0.status, new_smart_farm.status);
                                    smart_farm_storage_guard
                                        .update(
                                            new_smart_farm.id,
                                            Bundled(
                                                new_smart_farm,
                                                FinalizedTxOut(
                                                    output,
                                                    OutputRef::new(tx.hash, output_idx as u64),
                                                ),
                                            ),
                                        )
                                        .await
                                } else {
                                    info!(
                                        "[DAO-Handler] Got new smart farm with id {} and status {} ",
                                        hex::encode(new_smart_farm.id.0.as_bytes()),
                                        new_smart_farm.status
                                    );
                                    smart_farm_storage_guard
                                        .put(
                                            new_smart_farm.id,
                                            Bundled(
                                                new_smart_farm,
                                                FinalizedTxOut(
                                                    output,
                                                    OutputRef::new(tx.hash, output_idx as u64),
                                                ),
                                            ),
                                        )
                                        .await
                                }
                            }
                            OnChainEvent::BufferedWallet(new_buffered_wallet) => {
                                info!("[DAO-Handler] Got new buffered wallet {}", new_buffered_wallet);
                                buffered_wallet_storage_guard
                                    .put(
                                        OutputRef::new(tx.hash, output_idx as u64),
                                        Bundled(
                                            BufferedWallet {
                                                id: OutputRef::new(tx.hash, output_idx as u64),
                                                status: BufferedWalletStatus::Free,
                                                splash_amount: new_buffered_wallet.splash_amount,
                                                lovelace_amount: new_buffered_wallet.lovelace_amount,
                                            },
                                            FinalizedTxOut(
                                                output,
                                                OutputRef::new(tx.hash, output_idx as u64),
                                            ),
                                        ),
                                    )
                                    .await
                            }
                            OnChainEvent::PermManager(perm_manager) => {
                                info!("[DAO-Handler] Got perm manager entity {:?}", perm_manager);
                                perm_manager_storage_guard
                                    .put(
                                        OutputRef::new(tx.hash, output_idx as u64),
                                        Bundled(
                                            perm_manager,
                                            FinalizedTxOut(
                                                output,
                                                OutputRef::new(tx.hash, output_idx as u64),
                                            ),
                                        ),
                                    )
                                    .await
                            }
                        }
                    }
                }
            }
            LedgerTxEvent::TxUnapplied {
                tx,
                slot,
                block_number,
                block_hash,
            } => {}
        }

        Some(ev)
    }
}
