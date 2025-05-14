use crate::onchain::event::{BufferWalletAddress, OnChainEvent};
//use crate::pipeline::read_events::read_events;
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::certs::StakeCredential;
use cml_chain::transaction::{Transaction, TransactionOutput};
use cml_crypto::{ScriptHash, TransactionHash};
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::channel::mpsc::Sender;
use futures::FutureExt;
use futures::{Stream, StreamExt};
use log::info;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::domain::Has;
use spectrum_offchain::persistent_index::PersistentIndex;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::Snapshot;
use splash_dao_offchain::protocol_config::{FarmAuthPolicy, PermManagerAuthPolicy, SplashPolicy};
use splash_dao_offchain::routines::TimedOutputRef;
//use splash_distribution::config::config::HarvestLimits;
use splash_distribution::distributor::user_requests_processor::UserRequestsProcessor;
use splash_distribution::entities::buffered_wallet::BufferedWallet;
use splash_distribution::entities::events::user::withdraw::{UserWithdraw, UserWithdrawStatus};
use splash_distribution::entities::smart_farm::SmartFarm;
use splash_distribution::index::status_events_index::StatusEntitiesIndex;
//use splash_lp_indexer::onchain::event::OnChainEvent;
use splash_lp_indexer::position_db::accounts::Accounts;
use splash_lp_indexer::position_db::event_log::EventLog;
use splash_lp_indexer::position_db::pool_frames::PoolFrames;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

// pub async fn log_lp_events<U, UserWithdrawStorage, SmartFarmStorage, BufferedWalletStorage, Log>(
//     upstream: U,
//     user_withdraw_storage: Arc<Mutex<UserWithdrawStorage>>,
//     smart_farm_storage: Arc<Mutex<SmartFarmStorage>>,
//     buffered_wallet_storage: Arc<Mutex<BufferedWalletStorage>>,
//     log: &Log,
// ) where
//     U: Stream<Item = (BlockEvents<Snapshot<StatelessOnChainEvent, FinalizedTxOut>>, TransactionHandle)>,
//     UserWithdrawStorage: StatusEntitiesIndex<OutputRef, Snapshot<UserWithdraw, TransactionOutput>>,
//     SmartFarmStorage: StatusEntitiesIndex<OutputRef, Snapshot<SmartFarm, TransactionOutput>>,
//     BufferedWalletStorage: StatusEntitiesIndex<OutputRef, Snapshot<BufferedWallet, TransactionOutput>>,
//     Log: EventLog<StatelessOnChainEvent>,
// {
//     upstream
//         .for_each(|(block, transaction_handle)| {
//             let uw_guard = user_withdraw_storage.clone();
//             let sf_guard = smart_farm_storage.clone();
//             let bw_guard = buffered_wallet_storage.clone();
//             async move {
//                 info!("Logging event");
//                 log_event(block, log, uw_guard, sf_guard, bw_guard).await;
//                 info!("Going to call handle");
//                 transaction_handle.commit();
//             }
//         })
//         .await
// }
//
// // todo: to handlers?
// pub async fn log_events<U, Cx, Utxos, UserWithdrawStorage, SmartFarmStorage, BufferedWalletStorage, Log>(
//     upstream: U,
//     context: Cx,
//     utxos: Utxos,
//     events_log: Log,
//     utxo_filter: HashSet<ScriptHash>,
//     user_withdraw_storage: Arc<Mutex<UserWithdrawStorage>>,
//     smart_farm_storage: Arc<Mutex<SmartFarmStorage>>,
//     buffered_wallet_storage: Arc<Mutex<BufferedWalletStorage>>,
//     confirmed_txs: Sender<(TransactionHash, u64)>,
// ) where
//     U: Stream<
//         Item = (
//             BlockEvents<Either<BabbageTransaction, Transaction>>,
//             TransactionHandle,
//         ),
//     >,
//     Log: EventLog<StatelessOnChainEvent>,
//     UserWithdrawStorage: StatusEntitiesIndex<OutputRef, Snapshot<UserWithdraw, FinalizedTxOut>>,
//     SmartFarmStorage: StatusEntitiesIndex<OutputRef, Snapshot<SmartFarm, FinalizedTxOut>>,
//     BufferedWalletStorage: StatusEntitiesIndex<OutputRef, Snapshot<BufferedWallet, FinalizedTxOut>>,
//     Cx: Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
//         + Has<HarvestLimits>
//         + Has<NetworkId>
//         + Has<PermManagerAuthPolicy>
//         + Has<FarmAuthPolicy>
//         + Has<SplashPolicy>
//         + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
//         + Has<BufferWalletAddress>
//         + Clone,
//     Utxos: PersistentIndex<OutputRef, TransactionOutput>,
// {
//     log_lp_events(
//         upstream.then(|(block, tx_handle)| {
//             read_events(block, &context, &utxos, &utxo_filter, confirmed_txs.clone())
//                 .map(|block_events: BlockEvents<StatelessOnChainEvent>| (block_events, tx_handle))
//         }),
//         user_withdraw_storage,
//         smart_farm_storage,
//         buffered_wallet_storage,
//         &events_log,
//     )
//     .await
// }
//
// pub async fn log_event<Log, UserWithdrawStorage, SmartFarmStorage, BufferedWalletStorage>(
//     events: BlockEvents<Snapshot<StatelessOnChainEvent, FinalizedTxOut>>,
//     log: &Log,
//     user_withdraw_storage: Arc<Mutex<UserWithdrawStorage>>,
//     smart_farm_storage: Arc<Mutex<SmartFarmStorage>>,
//     buffered_wallet_storage: Arc<Mutex<BufferedWalletStorage>>,
// ) where
//     Log: EventLog<StatelessOnChainEvent>,
//     UserWithdrawStorage: StatusEntitiesIndex<OutputRef, Snapshot<UserWithdraw, FinalizedTxOut>>,
//     SmartFarmStorage: StatusEntitiesIndex<OutputRef, Snapshot<SmartFarm, FinalizedTxOut>>,
//     BufferedWalletStorage: StatusEntitiesIndex<OutputRef, Snapshot<BufferedWallet, FinalizedTxOut>>,
// {
//     match events {
//         BlockEvents::RollForward {
//             events, block_slot, ..
//         } => {
//             for event in events.clone() {
//                 match event.1.get() {
//                     StatelessOnChainEvent::MultipleHarvest(harvests) => {
//                         info!("Found harvest request: {}", harvests);
//                         let storage_guard = user_withdraw_storage.lock().await;
//                         let mut events: Vec<Snapshot<UserWithdraw, FinalizedTxOut>> = vec![];
//                         for harvest in harvests.accounts {
//                             let network_id = NetworkId::from(0);
//                             events.push(Snapshot(
//                                 UserWithdraw {
//                                     status: UserWithdrawStatus::New,
//                                     account: Address::Enterprise(EnterpriseAddress::new(
//                                         network_id.into(),
//                                         harvest,
//                                     )),
//                                 },
//                                 event.1.version().clone(),
//                             ))
//                         }
//
//                         for event in events {
//                             storage_guard.save_event(event).await
//                         }
//                     }
//                     StatelessOnChainEvent::FarmEvent(farm_event) => match farm_event {
//                         FarmEvents::FarmCreated(farm_created) => {}
//                         FarmEvents::FarmStateUpdate(farm_state_update) => {}
//                     },
//                     StatelessOnChainEvent::BufferedWallet(bf_wallet) => {}
//                 }
//             }
//         }
//         BlockEvents::RollBackward {
//             events, block_slot, ..
//         } => unimplemented!(), //log.batch_discard(block_slot, events).await,
//     }
// }
