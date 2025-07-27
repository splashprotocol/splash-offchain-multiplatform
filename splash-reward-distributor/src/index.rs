use crate::onchain::auth_manager::AuthManager;
use crate::onchain::buffer_wallet::BufferWallet;
use crate::onchain::harvest_order::HarvestOrder;
use crate::onchain::smart_farm::Gauge;
use async_trait::async_trait;
use bloom_offchain::execution_engine::bundled::Bundled;

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
}
