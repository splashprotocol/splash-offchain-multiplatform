pub(crate) mod rocksdb;

use std::fmt::{Debug, Display};
use std::hash::Hash;

use crate::entity_index::rocksdb::OnChainIndex;
use async_trait::async_trait;
use bloom_offchain::execution_engine::bundled::Bundled;
use cardano_chain_sync::atomic_flow::BlockEvents;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use spectrum_offchain::domain::event::{AnyMod, Confirmed, Predicted, Traced};
use splash_dao_offchain::funding::FundingRepo;
use splash_yf_offchain::entities::auth_manager::{AuthManager, AuthManagerId};
use splash_yf_offchain::entities::buffer_wallet::{BufferWallet, BufferWalletId};
use splash_yf_offchain::entities::funding_box::ConfirmedFundingBoxChanges;
use splash_yf_offchain::entities::harvest_order::HarvestOrder;
use splash_yf_offchain::entities::smart_farm::{Gauge, UpdatedGauges};
use splash_yf_offchain::events::OnChainEvent;

#[async_trait]
pub trait BufferWalletIndex<StateId, Bearer> {
    async fn get_buffer_wallet(&self) -> Option<Bundled<BufferWallet<StateId>, Bearer>>;
    async fn write_confirmed_buffer_wallet(
        &self,
        bundle: Bundled<BufferWallet<StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    );
    async fn write_predicted_buffer_wallet(
        &self,
        bundle: Bundled<BufferWallet<StateId>, Bearer>,
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

#[derive(Debug, PartialEq, Eq)]
pub enum Mod<T> {
    Confirmed(T),
    Predicted(T),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum HarvestOrderStatus {
    Spent,
    Unspent,
    Refunded,
}

#[async_trait::async_trait]
pub trait HarvestOrderIndex<StateId, Bearer>
where
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
    Bearer: Serialize + DeserializeOwned + 'static,
{
    async fn read_harvest_order(
        &self,
        id: StateId,
    ) -> Option<Mod<Bundled<(HarvestOrder<StateId>, HarvestOrderStatus), Bearer>>>;
    async fn write_confirmed_harvest_order(&self, order: Confirmed<Bundled<HarvestOrder<StateId>, Bearer>>);
    async fn write_predicted_spend_harvest_order(&self, id: StateId);
    async fn write_confirmed_spend_harvest_order(&self, id: StateId);
    async fn write_confirmed_refund_harvest_order(&self, id: StateId);
    /// Used on rollback of a spent or refunded order
    async fn unconsume_harvest_order(&self, id: StateId);
    /// Used on rollback of an unspent order
    async fn remove_created_harvest_order(&self, id: StateId);
}

pub async fn index_events<GaugeId, StateId, Bearer, I, F>(
    events: BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
    indexer: &I,
    funding: &F,
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
        BlockEvents::RollForward { events, .. } => {
            for event in events {
                match event {
                    OnChainEvent::BotHarvestingAction {
                        payouts,
                        buffer_wallet_update,
                    } => {
                        // Index new buffer_wallet state
                        let prev_state_id = buffer_wallet_update.consumed;
                        let (entity, bearer) = buffer_wallet_update.created.clone();
                        let bundled = Bundled(entity, bearer);
                        indexer
                            .write_confirmed_buffer_wallet(bundled, prev_state_id)
                            .await;

                        for (harvest_order, _) in payouts {
                            indexer
                                .write_confirmed_spend_harvest_order(harvest_order.id)
                                .await;
                        }
                    }
                    OnChainEvent::BotGaugeBufferingAction {
                        drained_gauges,
                        buffer_wallet_update,
                    } => {
                        // Index new buffer_wallet state
                        let prev_state_id = buffer_wallet_update.consumed;
                        let (entity, bearer) = buffer_wallet_update.created.clone();
                        let bundled = Bundled(entity, bearer);
                        indexer
                            .write_confirmed_buffer_wallet(bundled, prev_state_id)
                            .await;

                        // Index drained gauges
                        for gauge_update in drained_gauges {
                            let prev_state_id = gauge_update.consumed;
                            let (gauge, bearer) = gauge_update.created.clone();
                            let bundled = Bundled(gauge, bearer);
                            indexer.write_confirmed_gauge(bundled, prev_state_id).await;
                        }
                    }
                    OnChainEvent::UpdatedGauges(UpdatedGauges(updated_gauges)) => {
                        for gauge_update in updated_gauges {
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
        BlockEvents::RollBackward { events, .. } => {
            for event in events {
                match event {
                    OnChainEvent::BotHarvestingAction {
                        payouts,
                        buffer_wallet_update,
                    } => {
                        let prev_state_id = indexer
                            .remove_buffer_wallet(buffer_wallet_update.created.0.state_id)
                            .await;
                        assert_eq!(buffer_wallet_update.consumed, prev_state_id);
                        for (harvest_order, _) in payouts {
                            indexer.unconsume_harvest_order(harvest_order.id).await;
                        }
                    }
                    OnChainEvent::BotGaugeBufferingAction {
                        drained_gauges,
                        buffer_wallet_update,
                    } => {
                        let prev_state_id = indexer
                            .remove_buffer_wallet(buffer_wallet_update.created.0.state_id)
                            .await;
                        assert_eq!(buffer_wallet_update.consumed, prev_state_id);

                        for gauge_update in drained_gauges {
                            let gauge_id = gauge_update.created.0.id;
                            let prev_state_id = indexer
                                .remove_gauge(gauge_id, gauge_update.created.0.state_id)
                                .await;
                            assert_eq!(gauge_update.consumed, prev_state_id);
                        }
                    }
                    OnChainEvent::UpdatedGauges(UpdatedGauges(updated_gauges)) => {
                        for gauge_update in updated_gauges {
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
                        indexer.remove_created_harvest_order(harvest_order.id).await;
                    }
                    OnChainEvent::HarvestRequestCancelled(harvest_ids) => {
                        for harvest_id in harvest_ids {
                            indexer.unconsume_harvest_order(*harvest_id).await;
                        }
                    }
                    OnChainEvent::Funding(funding_updates) => {
                        let ConfirmedFundingBoxChanges { consumed, created } = funding_updates;
                        for id in consumed {
                            funding.unspend_confirmed(id.clone()).await;
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

#[async_trait]
impl<StateId, Bearer, T> BufferWalletIndex<StateId, Bearer> for T
where
    T: OnChainIndex<Bearer> + Send + Sync,
    StateId: Send + Sync + Debug + Display + Copy + Hash + Serialize + DeserializeOwned + Eq + 'static,
    Bearer: Serialize + DeserializeOwned + Send + 'static,
{
    async fn get_buffer_wallet(&self) -> Option<Bundled<BufferWallet<StateId>, Bearer>> {
        self.read::<BufferWallet<_>>(BufferWalletId)
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
        bundled: Bundled<BufferWallet<StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    ) {
        let traced = Traced::new(Confirmed(bundled), prev_state_id);
        self.write_confirmed(traced).await;
    }

    async fn write_predicted_buffer_wallet(
        &self,
        bundled: Bundled<BufferWallet<StateId>, Bearer>,
        prev_state_id: Option<StateId>,
    ) {
        let traced = Traced::new(Predicted(bundled), prev_state_id);
        self.write_predicted(traced).await;
    }

    async fn remove_buffer_wallet(&self, id: StateId) -> Option<StateId> {
        self.remove::<BufferWallet<_>>(BufferWalletId, id).await
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
