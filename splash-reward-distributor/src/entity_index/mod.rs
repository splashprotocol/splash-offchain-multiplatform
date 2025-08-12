use std::fmt::{Debug, Display};
use std::hash::Hash;

use crate::indexer::OnChainIndex;
use crate::onchain::auth_manager::AuthManagerId;
use crate::onchain::buffer_wallet::{BufferWallet, BufferWalletId};
use crate::onchain::harvest_order::HarvestOrder;
use crate::onchain::smart_farm::{Gauge, UpdatedGauges};
use crate::{events::OnChainEvent, onchain::auth_manager::AuthManager};
use async_trait::async_trait;
use bloom_offchain::execution_engine::bundled::Bundled;
use cardano_chain_sync::atomic_flow::BlockEvents;
use futures::Stream;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use spectrum_offchain::domain::event::{Confirmed, Traced};

#[async_trait]
pub trait BufferWalletIndex<StateId, Bearer> {
    async fn get_buffer_wallet(&self) -> Option<Bundled<BufferWallet<StateId>, Bearer>>;
}

#[async_trait]
pub trait OrderIndex<StateId, Bearer> {
    async fn get_order(&self, id: StateId) -> Option<Bundled<HarvestOrder<StateId>, Bearer>>;
}

#[async_trait]
pub trait GaugeIndex<GaugeId, StateId, Bearer> {
    async fn get_gauge(&self, id: GaugeId) -> Option<Bundled<Gauge<GaugeId, StateId>, Bearer>>;
    async fn get_auth_manager(&self) -> Option<Bundled<AuthManager<GaugeId, StateId>, Bearer>>;
    fn stream_gauges(&self) -> impl Stream<Item = Bundled<Gauge<GaugeId, StateId>, Bearer>>;
}

#[async_trait]
pub trait FundingBoxIndex<Bearer> {
    async fn get_funding_boxes(&self, lovelaces: u64) -> Vec<Bearer>;
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

pub async fn index_entities<GaugeId, StateId, Bearer, I>(
    events: BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
    indexer: I,
) where
    GaugeId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
    StateId: Copy + Eq + Hash + Send + Sync + Display + Debug + Serialize + DeserializeOwned + 'static,
    I: HarvestOrderIndex<StateId, Bearer> + OnChainIndex<Bearer>,
    Bearer: Serialize + DeserializeOwned + 'static,
{
    match events {
        BlockEvents::RollForward { events, .. } => {
            for event in events {
                match event {
                    OnChainEvent::BotHarvestingAction {
                        payouts,
                        buffer_wallet_update,
                    } => {
                        // Index new buffer_wallet state
                        let prev_state_id = buffer_wallet_update.consumed;
                        let (entity, bearer) = buffer_wallet_update.created;
                        let bundled = Bundled(entity, bearer);
                        let traced = Traced::new(Confirmed(bundled), prev_state_id);
                        indexer.write_confirmed(traced).await;

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
                        let (entity, bearer) = buffer_wallet_update.created;
                        let bundled = Bundled(entity, bearer);
                        let traced = Traced::new(Confirmed(bundled), prev_state_id);
                        indexer.write_confirmed(traced).await;

                        // Index drained gauges
                        for gauge_update in drained_gauges {
                            let prev_state_id = gauge_update.consumed;
                            let (gauge, bearer) = gauge_update.created;
                            let bundled = Bundled(gauge, bearer);
                            let traced = Traced::new(Confirmed(bundled), prev_state_id);
                            indexer.write_confirmed(traced).await;
                        }
                    }
                    OnChainEvent::UpdatedGauges(UpdatedGauges(updated_gauges)) => {
                        for gauge_update in updated_gauges {
                            let prev_state_id = gauge_update.consumed;
                            let (entity, bearer) = gauge_update.created;
                            let bundled = Bundled(entity, bearer);
                            let traced = Traced::new(Confirmed(bundled), prev_state_id);
                            indexer.write_confirmed(traced).await;
                        }
                    }
                    OnChainEvent::AuthManagerUpdated(auth_update) => {
                        let prev_state_id = auth_update.consumed;
                        let (entity, bearer) = auth_update.created;
                        let bundled = Bundled(entity, bearer);
                        let traced = Traced::new(Confirmed(bundled), prev_state_id);
                        indexer.write_confirmed(traced).await;
                    }
                    OnChainEvent::NewHarvestRequest(harvest, output) => {
                        let order = Confirmed(Bundled(harvest, output));
                        indexer.write_confirmed_harvest_order(order).await;
                    }
                    OnChainEvent::HarvestRequestCancelled(harvest_ids) => {
                        for id in harvest_ids {
                            indexer.write_confirmed_refund_harvest_order(id).await;
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
                            .remove::<BufferWallet<_>>(
                                BufferWalletId,
                                buffer_wallet_update.created.0.state_id,
                            )
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
                            .remove::<BufferWallet<_>>(
                                BufferWalletId,
                                buffer_wallet_update.created.0.state_id,
                            )
                            .await;
                        assert_eq!(buffer_wallet_update.consumed, prev_state_id);

                        for gauge_update in drained_gauges {
                            let gauge_id = gauge_update.created.0.id;
                            let prev_state_id = indexer
                                .remove::<Gauge<_, _>>(gauge_id, gauge_update.created.0.state_id)
                                .await;
                            assert_eq!(gauge_update.consumed, prev_state_id);
                        }
                    }
                    OnChainEvent::UpdatedGauges(UpdatedGauges(updated_gauges)) => {
                        for gauge_update in updated_gauges {
                            let prev_state_id = indexer
                                .remove::<Gauge<_, _>>(
                                    gauge_update.created.0.id,
                                    gauge_update.created.0.state_id,
                                )
                                .await;
                            assert_eq!(gauge_update.consumed, prev_state_id);
                        }
                    }
                    OnChainEvent::AuthManagerUpdated(auth_update) => {
                        let prev_state_id = indexer
                            .remove::<AuthManager<GaugeId, StateId>>(
                                AuthManagerId,
                                auth_update.created.0.state_id,
                            )
                            .await;
                        assert_eq!(auth_update.consumed, prev_state_id);
                    }
                    OnChainEvent::NewHarvestRequest(harvest_order, _) => {
                        indexer.remove_created_harvest_order(harvest_order.id).await;
                    }
                    OnChainEvent::HarvestRequestCancelled(harvest_ids) => {
                        for harvest_id in harvest_ids {
                            indexer.unconsume_harvest_order(harvest_id).await;
                        }
                    }
                }
            }
        }
    }
}
