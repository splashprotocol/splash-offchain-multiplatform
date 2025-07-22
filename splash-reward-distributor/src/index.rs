use async_trait::async_trait;
use spectrum_offchain::domain::order::UniqueOrder;
use spectrum_offchain::domain::EntitySnapshot;

#[async_trait]
pub trait OnChainIndex<P: EntitySnapshot, O: UniqueOrder> {
    async fn get_entity(&self, id: P::StableId) -> Option<P>;
    async fn get_order(&self, id: O::TOrderId) -> Option<O>;
}
