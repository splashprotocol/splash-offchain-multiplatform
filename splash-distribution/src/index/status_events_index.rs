use async_trait::async_trait;
use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::output::FinalizedTxOut;
use splash_dao_offchain::entities::HasStatus;

#[async_trait]
pub trait StatusEntitiesIndex<K, V: HasStatus> {

    fn status_entity_key(key: K, entity: Bundled<V, FinalizedTxOut>) -> Vec<u8>;

    async fn get_events_by_status(&self, status: <V as HasStatus>::Status) -> Vec<Bundled<V, FinalizedTxOut>>;

    async fn get_event_by_key(&self, key: K) -> Option<Bundled<V, FinalizedTxOut>>;

    async fn drop_event(&self, event_key: K);

    async fn put(&self, key: K, event: Bundled<V, FinalizedTxOut>);

    async fn update(&self, key: K, event: Bundled<V, FinalizedTxOut>);

    async fn update_event_status(&self, event_key: K, status: <V as HasStatus>::Status);
}
