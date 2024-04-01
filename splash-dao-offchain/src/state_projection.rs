use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_offchain::data::unique_entity::{AnyMod, Predicted, Traced};
use spectrum_offchain::data::{EntitySnapshot, Identifier};

/// Projection of [T] state relative to the ledger.
#[async_trait::async_trait]
pub trait StateProjectionRead<T, B>
where
    T: EntitySnapshot,
{
    async fn read<I>(&self, id: I) -> Option<AnyMod<Bundled<T, B>>>
    where
        I: Identifier<For = T> + Send;
}

#[async_trait::async_trait]
pub trait StateProjectionWrite<T, B>
where
    T: EntitySnapshot,
{
    async fn write(&self, entity: Traced<Predicted<Bundled<T, B>>>);
}
