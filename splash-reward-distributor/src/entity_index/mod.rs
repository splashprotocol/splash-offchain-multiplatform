pub(crate) mod chained_tx_graph;
pub(crate) mod rocksdb;

use std::collections::HashSet;
use std::fmt::{Debug, Display};
use std::hash::Hash;

use crate::accounts::{Accounts, PositionIndex};
use crate::entity_index::rocksdb::OnChainIndex;
use async_trait::async_trait;
use bloom_offchain::execution_engine::bundled::Bundled;
use cardano_chain_sync::atomic_flow::BlockEvents;
use cml_chain::address::Address;
use cml_chain::certs::Credential;
use cml_chain::transaction::Transaction;
use cml_crypto::{Ed25519KeyHash, TransactionHash};
use futures::channel::mpsc::{Receiver, Sender};
use futures::{SinkExt, StreamExt};
use rs_merkle::algorithms::Keccak256;
use rs_merkle::{Hasher, MerkleTree};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::tx_view::TimedOutput;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::domain::event::{AnyMod, Confirmed, Predicted, Traced};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::persistent_index::PersistentIndex;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::onchain::funding_box::FundingBoxSnapshot;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::funding::FundingRepo;
use splash_dao_offchain::protocol_config::{
    BufferWalletAuthPolicy, OperatorCreds, PermManagerAuthPolicy, SplashPolicy,
};
use splash_dao_offchain::routines::{slot_to_epoch, Slot, TimedOutputRef};
use splash_dao_offchain::GenesisEpochStartTime;
use splash_yf_offchain::entities::auth_manager::{AuthManager, AuthManagerId};
use splash_yf_offchain::entities::buffer_wallet::{
    try_extract_buffer_wallet, BufferWallet, BufferWalletId, BufferWalletWrap,
};
use splash_yf_offchain::entities::funding_box::ConfirmedFundingBoxChanges;
use splash_yf_offchain::entities::gauge::{
    try_extract_gauge, Gauge, GaugeDeposits, GaugeWithdrawals, UpdatedGauges,
};
use splash_yf_offchain::entities::harvest_order::{try_extract_harvest_order, HarvestOrder};
use splash_yf_offchain::events::{OnChainEvent, SplashPayout};
use splash_yf_offchain::settings::MinLovelacePerHarvest;
use splash_yf_offchain::Epoch;
use type_equalities::IsEqual;

#[async_trait]
pub trait BufferWalletIndex<StateId, Bearer> {
    async fn get_buffer_wallet(&self) -> Option<Bundled<BufferWalletWrap<StateId>, Bearer>>;
    async fn write_confirmed_buffer_wallet(
        &self,
        bundle: Bundled<BufferWalletWrap<StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    );
    async fn write_predicted_buffer_wallet(
        &self,
        bundle: Bundled<BufferWalletWrap<StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    );
    async fn remove_buffer_wallet(&self, id: StateId) -> Option<StateId>;
}

#[async_trait]
pub trait GaugeIndex<GaugeId, StateId, Bearer> {
    async fn get_gauge(&self, id: GaugeId) -> Option<Bundled<Gauge<GaugeId, StateId>, Bearer>>;
    async fn write_confirmed_gauge(
        &self,
        bundle: Bundled<Gauge<GaugeId, StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    );
    async fn write_predicted_gauge(
        &self,
        bundle: Bundled<Gauge<GaugeId, StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    );
    async fn remove_gauge(&self, gauge_id: GaugeId, state_id: StateId) -> Option<StateId>;
}

#[async_trait]
pub trait AuthManagerIndex<GaugeId, StateId, Bearer> {
    async fn get_auth_manager(&self) -> Option<Bundled<AuthManager<GaugeId, StateId>, Bearer>>;
    async fn write_confirmed_auth_manager(
        &self,
        bundle: Bundled<AuthManager<GaugeId, StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    );
    async fn remove_auth_manager(&self, state_id: StateId) -> Option<StateId>;
}

pub trait UnconfirmedHarvestTxIndex {
    /// Try adding a harvest TX to the index, returning true if successful.
    ///
    /// `buffer_wallet_input_tx_hash` must refer to a TX hash of a confirmed buffering/harvest
    /// action or an unconfirmed harvest operation that has **already been** cosigned by this
    /// verifier and belongs to the index.
    ///
    /// **IMPORTANT NODE:** A TX that has been cosigned by another verifier will not be accepted
    /// into this index.
    ///
    /// `tx_user_creds` is a Vec of credentials of users who are harvesting their rewards in this
    /// TX. It will be checked against existing TXs to ensure that double-harvesting does not occur.
    fn try_add_tx(
        &mut self,
        buffer_wallet_input_tx_hash: TransactionHash,
        tx_hash: TransactionHash,
        tx_user_creds: HashSet<Ed25519KeyHash>,
    ) -> bool;

    /// If the chain experiences a rollback which leads to a change in the last-confirmed
    /// `buffer_wallet` UTxO, this method is called to sync the index accordingly. Note that
    /// `rolled_back_user_creds` denotes the users who have harvested rewards in the TX that is
    /// being rolled back and `confirmed_buffer_wallet_tx_hash` denotes the hash of the last-confirmed
    /// TX AFTER the rollback.
    fn rollback(
        &mut self,
        rolled_back_user_creds: HashSet<Ed25519KeyHash>,
        confirmed_buffer_wallet_tx_hash: TransactionHash,
    );

    /// Confirms the TX with the given TX-hash. There are 2 possibilities:
    /// 1. The TX is already in the index i.e. with unconfirmed state. Return true.
    /// 2. The TX is either a gauge-buffering action, or it was signed by another verifier. For the
    ///    latter case it is essential to be given a Vec of `confirmed_user_harvests` for this
    ///    epoch. Return false.
    ///
    /// NOTE: `confirmed_user_harvests` denotes users who have harvested rewards in this confirmed
    /// TX only.
    fn confirm_tx(
        &mut self,
        tx_hash: TransactionHash,
        confirmed_user_harvests: &HashSet<Ed25519KeyHash>,
    ) -> bool;

    /// Upon the end of an epoch, the index will delete all its unconfirmed TXs.
    fn notify_end_of_epoch(&mut self);
}

#[derive(Debug, PartialEq, Eq)]
pub enum Mod<T> {
    Confirmed(T),
    Predicted(T),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum HarvestOrderStatus {
    Spent(Epoch),
    Unspent,
    Refunded,
}

/// Index of harvest orders. Note that the bot does not allow for multiple orders per user per
/// epoch. This index **designates** one order per user per epoch, and ignores subsequent orders.
///
/// **NOTE:**
/// - The only way a user can lose a designated order for an epoch is if they refund it. In
///   particular, the designation remains even when the harvest order is spent.
/// - If a designated harvest-order that was refunded is later rolled-back on chain (so that it is
///   available to be spent again), the order will regain its designated status.
/// - Suppose that a user creates 2 harvest orders A then B in the same epoch within separate TXs.
///   A is the designated order and B will be ignored. Crucially, if A is refunded by the user
///   before it is processed by the reward-bot, order B WILL NOT be designated.
#[async_trait::async_trait]
pub trait HarvestOrderIndex<StateId, Bearer>
where
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
    Bearer: Serialize + DeserializeOwned + 'static,
{
    /// Get the designated harvest order for a given state ID if it exists.
    async fn read_designated_harvest_order(
        &self,
        id: StateId,
    ) -> Option<Mod<Bundled<(HarvestOrder<StateId>, HarvestOrderStatus), Bearer>>>;

    /// Write a confirmed harvest order, but note that this method no-ops if an existing order is
    /// already designated for the epoch.
    async fn write_confirmed_harvest_order(&self, order: Confirmed<Bundled<HarvestOrder<StateId>, Bearer>>);
    async fn write_predicted_spend_harvest_order(&self, id: StateId, predicted_spend: &HarvestOrderSpend);
    /// Write a confirmed spending of all harvest orders withing a harvest-order TX. All spendings
    /// are processed in a batch since we also need to update the merkle tree.
    async fn write_confirmed_spend_harvest_orders(
        &self,
        orders: Vec<(StateId, HarvestOrderSpend)>,
        confirmed_slot: u64,
    );
    /// Confirm a refund of a harvest order.
    async fn write_confirmed_refund_harvest_order(&self, id: StateId);
    /// Undo a confirmed refund of a harvest order, which occurs on chain-rollback.
    async fn undo_confirmed_refund_harvest_order(&self, id: StateId);
    /// Used on rollback of a harvest-order TX.
    async fn undo_confirmed_spent_harvest_orders(
        &self,
        spent_orders: Vec<(Ed25519KeyHash, StateId)>,
        confirmed_slot: u64,
    );
    /// Undo a predicted spending of a harvest order, which occurs when a harvest-order TX is
    /// dropped from the mempool.
    async fn undo_predicted_spent_harvest_order(&self, id: StateId);
    /// Used on rollback of an unspent order
    async fn remove_created_harvest_order(&self, id: StateId, user: Ed25519KeyHash);
    /// Get the last epoch a user harvested rewards from the DAO.
    async fn last_epoch_harvested(&self, user: Ed25519KeyHash) -> Option<Epoch>;
    async fn last_confirmed_merkle_tree(&self) -> Option<IndexedMerkleTree>;
}

#[derive(Clone, Serialize, Deserialize)]
pub struct IndexedMerkleTree {
    pub tree: MerkleTree<Keccak256>,
    pub slot: Slot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HarvestOrderSpend {
    pub amount: u64,
    pub epoch_start: Epoch,
    pub epoch_end: Epoch,
    pub user: Ed25519KeyHash,
    pub reward_address: Address,
}

impl HarvestOrderSpend {
    pub fn hash(&self) -> [u8; 32] {
        use cml_crypto::RawBytesEncoding;
        let mut data = self.amount.to_be_bytes().to_vec();
        data.extend_from_slice(u64::from(self.epoch_start).to_be_bytes().as_ref());
        data.extend_from_slice(u64::from(self.epoch_end).to_be_bytes().as_ref());
        data.extend_from_slice(self.user.to_raw_bytes());
        data.extend_from_slice(self.reward_address.to_raw_bytes().as_ref());

        Keccak256::hash(&data)
    }
}

pub async fn index_events<GaugeId, StateId, Bearer, I, F>(
    events: BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
    indexer: &I,
    funding: &F,
    genesis_start_time: GenesisEpochStartTime,
    network_id: NetworkId,
) -> BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>
where
    GaugeId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
    StateId: Copy + Eq + Hash + Send + Sync + Display + Debug + Serialize + DeserializeOwned + 'static,
    I: HarvestOrderIndex<StateId, Bearer>
        + BufferWalletIndex<StateId, Bearer>
        + GaugeIndex<GaugeId, StateId, Bearer>
        + AuthManagerIndex<GaugeId, StateId, Bearer>,
    Bearer: Clone + Serialize + DeserializeOwned + 'static,
    F: FundingRepo + Clone,
{
    match &events {
        BlockEvents::RollForward {
            events, block_slot, ..
        } => {
            for event in events {
                match event {
                    OnChainEvent::BotHarvestingAction {
                        payouts,
                        buffer_wallet_update,
                        ..
                    } => {
                        // Index new buffer_wallet state
                        let prev_state_id = buffer_wallet_update.consumed;
                        let (wallet, bearer) = buffer_wallet_update.created.clone();
                        let entity = BufferWalletWrap {
                            wallet,
                            predicted_merkle_tree: None,
                        };
                        let bundled = Bundled(entity, bearer);
                        indexer
                            .write_confirmed_buffer_wallet(bundled, prev_state_id)
                            .await;

                        let epoch_end_inner =
                            slot_to_epoch(*block_slot, genesis_start_time, network_id).0 as u64;
                        assert!(epoch_end_inner > 0);
                        let epoch_end = Epoch::from(epoch_end_inner - 1);

                        let mut orders = vec![];
                        for (harvest_order, SplashPayout(amount)) in payouts {
                            let epoch_start = indexer
                                .last_epoch_harvested(harvest_order.account_key)
                                .await
                                .map(|e| e.next())
                                .unwrap_or(Epoch::from(0));
                            assert!(epoch_start <= epoch_end);

                            let confirmed_spend = HarvestOrderSpend {
                                amount: *amount,
                                epoch_start,
                                epoch_end,
                                user: harvest_order.account_key,
                                reward_address: harvest_order.reward_receiver.to_address(network_id),
                            };
                            orders.push((harvest_order.id, confirmed_spend));
                        }
                        indexer
                            .write_confirmed_spend_harvest_orders(orders, *block_slot)
                            .await;
                    }
                    OnChainEvent::BotGaugeBufferingAction {
                        drained_gauges: GaugeWithdrawals(drained_gauges),
                        buffer_wallet_update,
                        ..
                    } => {
                        // Index new buffer_wallet state
                        let prev_state_id = buffer_wallet_update.consumed;
                        let (wallet, bearer) = buffer_wallet_update.created.clone();
                        let entity = BufferWalletWrap {
                            wallet,
                            predicted_merkle_tree: None,
                        };
                        let bundled = Bundled(entity, bearer);
                        indexer
                            .write_confirmed_buffer_wallet(bundled, prev_state_id)
                            .await;

                        // Index drained gauges
                        for (gauge_update, _) in drained_gauges {
                            let prev_state_id = gauge_update.consumed;
                            let (gauge, bearer) = gauge_update.created.clone();
                            let bundled = Bundled(gauge, bearer);
                            indexer.write_confirmed_gauge(bundled, prev_state_id).await;
                        }
                    }
                    OnChainEvent::DepositToGauges(GaugeDeposits(updated_gauges)) => {
                        for (gauge_update, _) in updated_gauges {
                            let prev_state_id = gauge_update.consumed;
                            let (entity, bearer) = gauge_update.created.clone();
                            let bundled = Bundled(entity, bearer);
                            indexer.write_confirmed_gauge(bundled, prev_state_id).await;
                        }
                    }
                    OnChainEvent::AuthManagerUpdated(auth_update) => {
                        let prev_state_id = auth_update.consumed;
                        let (entity, bearer) = auth_update.created.clone();
                        let bundled = Bundled(entity, bearer);
                        indexer.write_confirmed_auth_manager(bundled, prev_state_id).await;
                    }
                    OnChainEvent::NewHarvestRequest(harvest, output) => {
                        let order = Confirmed(Bundled(harvest.clone(), output.clone()));
                        indexer.write_confirmed_harvest_order(order).await;
                    }
                    OnChainEvent::HarvestRequestCancelled(harvest_ids) => {
                        for id in harvest_ids {
                            indexer.write_confirmed_refund_harvest_order(*id).await;
                        }
                    }
                    OnChainEvent::Funding(funding_updates) => {
                        let ConfirmedFundingBoxChanges { consumed, created } = funding_updates;
                        for id in consumed {
                            funding.spend_confirmed(*id).await;
                        }

                        for f in created {
                            funding.put_confirmed(Confirmed(f.clone())).await;
                        }
                    }
                }
            }
        }
        BlockEvents::RollBackward {
            events, block_slot, ..
        } => {
            for event in events {
                match event {
                    OnChainEvent::BotHarvestingAction {
                        payouts,
                        buffer_wallet_update,
                        ..
                    } => {
                        let prev_state_id = indexer
                            .remove_buffer_wallet(buffer_wallet_update.created.0.state_id)
                            .await;
                        assert_eq!(buffer_wallet_update.consumed, prev_state_id);
                        let spent_orders: Vec<_> = payouts
                            .iter()
                            .map(|(order, _)| (order.account_key, order.id))
                            .collect();
                        indexer
                            .undo_confirmed_spent_harvest_orders(spent_orders, *block_slot)
                            .await;
                    }
                    OnChainEvent::BotGaugeBufferingAction {
                        drained_gauges,
                        buffer_wallet_update,
                        ..
                    } => {
                        let prev_state_id = indexer
                            .remove_buffer_wallet(buffer_wallet_update.created.0.state_id)
                            .await;
                        assert_eq!(buffer_wallet_update.consumed, prev_state_id);

                        for (gauge_update, _) in &drained_gauges.0 {
                            let gauge_id = gauge_update.created.0.id;
                            let prev_state_id = indexer
                                .remove_gauge(gauge_id, gauge_update.created.0.state_id)
                                .await;
                            assert_eq!(gauge_update.consumed, prev_state_id);
                        }
                    }
                    OnChainEvent::DepositToGauges(GaugeDeposits(updated_gauges)) => {
                        for (gauge_update, _) in updated_gauges {
                            let prev_state_id = indexer
                                .remove_gauge(gauge_update.created.0.id, gauge_update.created.0.state_id)
                                .await;
                            assert_eq!(gauge_update.consumed, prev_state_id);
                        }
                    }
                    OnChainEvent::AuthManagerUpdated(auth_update) => {
                        let prev_state_id = indexer.remove_auth_manager(auth_update.created.0.state_id).await;
                        assert_eq!(auth_update.consumed, prev_state_id);
                    }
                    OnChainEvent::NewHarvestRequest(harvest_order, _) => {
                        indexer
                            .remove_created_harvest_order(harvest_order.id, harvest_order.account_key)
                            .await;
                    }
                    OnChainEvent::HarvestRequestCancelled(harvest_ids) => {
                        for harvest_id in harvest_ids {
                            indexer.undo_confirmed_refund_harvest_order(*harvest_id).await;
                        }
                    }
                    OnChainEvent::Funding(funding_updates) => {
                        let ConfirmedFundingBoxChanges { consumed, created } = funding_updates;
                        for id in consumed {
                            funding.unspend_confirmed(*id).await;
                        }

                        for f in created {
                            funding.eliminate_confirmed(f.id).await;
                        }
                    }
                }
            }
        }
    }
    events
}

pub async fn update_index_from_mempool_dropped_tx<OnChainIndex, Utxos, Ctx, FB>(
    recv: Receiver<Transaction>,
    send: Sender<TransactionHash>,
    index: OnChainIndex,
    funding: FB,
    utxos: Utxos,
    ctx: Ctx,
) where
    OnChainIndex: HarvestOrderIndex<OutputRef, FinalizedTxOut>
        + BufferWalletIndex<OutputRef, FinalizedTxOut>
        + GaugeIndex<FarmId, OutputRef, FinalizedTxOut>
        + AuthManagerIndex<FarmId, OutputRef, FinalizedTxOut>
        + Clone,
    Utxos: PersistentIndex<OutputRef, TimedOutput> + Clone,
    Ctx: Has<PermManagerAuthPolicy>
        + Has<OperatorCreds>
        + Has<SplashPolicy>
        + Has<MinLovelacePerHarvest>
        + Has<BufferWalletAuthPolicy>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::BufferWallet as u8 }>>,
    FB: FundingRepo + Send + Sync,
{
    recv.for_each(|tx| async {
        let tx_hash = tx.body.hash();

        send.clone().send(tx_hash).await.ok();

        for input in tx.body.inputs {
            let output_ref = OutputRef::from(input);
            let funding_ctx = FundingCtx {
                creds: ctx.select::<OperatorCreds>(),
                output_ref,
            };
            if let Some(TimedOutput { output, .. }) = utxos.get(output_ref).await {
                // We don't need the actual slot and epoch value to parse the following entity; a dummy value suffices
                if try_extract_harvest_order(&output, output_ref, (Slot(100), Epoch::from(0)), &ctx).is_some()
                {
                    index.undo_predicted_spent_harvest_order(output_ref).await;
                } else if FundingBoxSnapshot::try_from_ledger(&output, &funding_ctx).is_some() {
                    funding.unspend_predicted(output_ref.into()).await;
                }
            }
        }

        for (ix, output) in tx.body.outputs.into_iter().enumerate() {
            let output_ref = OutputRef::new(tx_hash, ix as u64);
            // Again, it's fine to have a dummy slot value
            let timed_output_ref = TimedOutputRef::new(output_ref, Slot(100));

            let funding_ctx = FundingCtx {
                creds: ctx.select::<OperatorCreds>(),
                output_ref,
            };
            if let Some(gauge) = try_extract_gauge(&output, timed_output_ref, &ctx) {
                index.remove_gauge(gauge.id, output_ref).await;
            } else if try_extract_buffer_wallet(&output, output_ref, &ctx).is_some() {
                index.remove_buffer_wallet(output_ref).await;
            } else if FundingBoxSnapshot::try_from_ledger(&output, &funding_ctx).is_some() {
                funding.eliminate_predicted(output_ref.into()).await;
            }
        }
    })
    .await;
}

struct FundingCtx {
    creds: OperatorCreds,
    output_ref: OutputRef,
}

impl Has<OutputRef> for FundingCtx {
    fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.output_ref
    }
}

impl Has<OperatorCreds> for FundingCtx {
    fn select<U: IsEqual<OperatorCreds>>(&self) -> OperatorCreds {
        self.creds.clone()
    }
}

#[async_trait]
impl<StateId, Bearer, T> BufferWalletIndex<StateId, Bearer> for T
where
    T: OnChainIndex<Bearer> + Send + Sync,
    StateId: Send + Sync + Debug + Display + Copy + Hash + Serialize + DeserializeOwned + Eq + 'static,
    Bearer: Serialize + DeserializeOwned + Send + 'static,
{
    async fn get_buffer_wallet(&self) -> Option<Bundled<BufferWalletWrap<StateId>, Bearer>> {
        self.read::<BufferWalletWrap<_>>(BufferWalletId)
            .await
            .map(|bw| match bw {
                AnyMod::Confirmed(Traced {
                    state: Confirmed(b), ..
                })
                | AnyMod::Predicted(Traced {
                    state: Predicted(b), ..
                }) => b,
            })
    }

    async fn write_confirmed_buffer_wallet(
        &self,
        bundled: Bundled<BufferWalletWrap<StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    ) {
        let traced = Traced::new(Confirmed(bundled), prev_state_id);
        self.write_confirmed(traced).await;
    }

    async fn write_predicted_buffer_wallet(
        &self,
        bundled: Bundled<BufferWalletWrap<StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    ) {
        let traced = Traced::new(Predicted(bundled), prev_state_id);
        self.write_predicted(traced).await;
    }

    async fn remove_buffer_wallet(&self, id: StateId) -> Option<StateId> {
        self.remove::<BufferWalletWrap<_>>(BufferWalletId, id).await
    }
}

#[async_trait]
impl<GaugeId, StateId, Bearer, T> GaugeIndex<GaugeId, StateId, Bearer> for T
where
    T: OnChainIndex<Bearer> + Send + Sync,
    GaugeId: Send + Sync + Debug + Display + Copy + Hash + Serialize + DeserializeOwned + Eq + 'static,
    StateId: Send + Sync + Debug + Display + Copy + Hash + Serialize + DeserializeOwned + Eq + 'static,
    Bearer: Serialize + DeserializeOwned + Send + 'static,
{
    async fn get_gauge(&self, id: GaugeId) -> Option<Bundled<Gauge<GaugeId, StateId>, Bearer>> {
        self.read::<Gauge<_, _>>(id).await.map(|bw| match bw {
            AnyMod::Confirmed(Traced {
                state: Confirmed(b), ..
            })
            | AnyMod::Predicted(Traced {
                state: Predicted(b), ..
            }) => b,
        })
    }

    async fn write_confirmed_gauge(
        &self,
        bundled: Bundled<Gauge<GaugeId, StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    ) {
        let traced = Traced::new(Confirmed(bundled), prev_state_id);
        self.write_confirmed(traced).await;
    }

    async fn write_predicted_gauge(
        &self,
        bundled: Bundled<Gauge<GaugeId, StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    ) {
        let traced = Traced::new(Predicted(bundled), prev_state_id);
        self.write_predicted(traced).await;
    }

    async fn remove_gauge(&self, gauge_id: GaugeId, state_id: StateId) -> Option<StateId> {
        self.remove::<Gauge<_, _>>(gauge_id, state_id).await
    }
}

#[async_trait]
impl<GaugeId, StateId, Bearer, T> AuthManagerIndex<GaugeId, StateId, Bearer> for T
where
    T: OnChainIndex<Bearer> + Send + Sync,
    GaugeId: Send + Sync + Debug + Display + Copy + Hash + Serialize + DeserializeOwned + Eq + 'static,
    StateId: Send + Sync + Debug + Display + Copy + Hash + Serialize + DeserializeOwned + Eq + 'static,
    Bearer: Serialize + DeserializeOwned + Send + 'static,
{
    async fn get_auth_manager(&self) -> Option<Bundled<AuthManager<GaugeId, StateId>, Bearer>> {
        self.read::<AuthManager<_, _>>(AuthManagerId)
            .await
            .map(|bw| match bw {
                AnyMod::Confirmed(Traced {
                    state: Confirmed(b), ..
                })
                | AnyMod::Predicted(Traced {
                    state: Predicted(b), ..
                }) => b,
            })
    }

    async fn write_confirmed_auth_manager(
        &self,
        bundled: Bundled<AuthManager<GaugeId, StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    ) {
        let traced = Traced::new(Confirmed(bundled), prev_state_id);
        self.write_confirmed(traced).await;
    }

    async fn remove_auth_manager(&self, state_id: StateId) -> Option<StateId> {
        self.remove::<AuthManager<GaugeId, StateId>>(AuthManagerId, state_id)
            .await
    }
}
