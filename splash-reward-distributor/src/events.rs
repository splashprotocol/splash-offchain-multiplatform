use crate::onchain::auth_manager::AuthManager;
use crate::onchain::buffer_wallet::BufferWallet;
use crate::onchain::harvest_order::HarvestOrder;
use crate::onchain::smart_farm::Gauge;
use cml_core::Slot;

#[derive(Debug, Clone, PartialEq)]
pub struct SettledEvent<GaugeId, StateId, Bearer>(OnChainEvent<GaugeId, StateId, Bearer>, Slot);

#[derive(Debug, Clone, PartialEq)]
pub enum OnChainEvent<GaugeId, StateId, Bearer> {
    NewHarvestRequest(HarvestOrder<StateId>),
    HarvestRequestCancelled(StateId),
    Harvested(StateId),
    BufferWalletUpdated(EntityUpdated<BufferWallet<StateId>, StateId, Bearer>),
    GaugeUpdated(EntityUpdated<Gauge<GaugeId, StateId>, StateId, Bearer>),
    AuthManagerUpdated(EntityUpdated<AuthManager<StateId>, StateId, Bearer>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityUpdated<Entity, StateId, Bearer> {
    consumed: Option<StateId>,
    created: (Entity, Bearer),
}
