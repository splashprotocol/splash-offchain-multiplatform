use std::io::Cursor;
use std::sync::Arc;

use async_std::task::spawn_blocking;
use bloom_offchain::execution_engine::bundled::Bundled;
use serde::de::DeserializeOwned;
use serde::Serialize;
use spectrum_offchain::data::unique_entity::{AnyMod, Confirmed, Predicted, Traced};
use spectrum_offchain::data::{EntitySnapshot, Identifier};

/// Projection of [T] state relative to the ledger.
#[async_trait::async_trait]
pub trait StateProjectionRead<T, I, B>
where
    T: EntitySnapshot,
    I: Identifier<For = T> + Send + Serialize,
{
    async fn read(&self, id: I) -> Option<AnyMod<Bundled<T, B>>>;
}

#[async_trait::async_trait]
pub trait StateProjectionWrite<T, I, B>
where
    T: EntitySnapshot,
    I: Identifier<For = T> + Send,
{
    async fn write_predicted(&self, entity: Traced<Predicted<Bundled<T, B>>>);
    async fn write_confirmed(&self, entity: Traced<Confirmed<Bundled<T, B>>>);
    async fn remove(&self, id: I) -> Option<I>;
}

const LATEST_VERSION_PREFIX: &str = "id:";
const PREVIOUS_VERSION_PREFIX: &str = "p_id:";
const STATE_PREFIX: &str = "s:";

pub struct StateProjectionRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

#[async_trait::async_trait]
impl<T, B, I> StateProjectionRead<T, I, B> for StateProjectionRocksDB
where
    T: EntitySnapshot + Send + Sync + Clone + Serialize + DeserializeOwned + 'static,
    T::Version: Send,
    B: Send + Sync + Clone + Serialize + DeserializeOwned + 'static,
    I: Identifier<For = T> + Send + Serialize + 'static,
{
    async fn read(&self, id: I) -> Option<AnyMod<Bundled<T, B>>> {
        let db = self.db.clone();
        let version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
        spawn_blocking(move || {
            db.get(version_key).unwrap().and_then(|version_bytes| {
                let mut bytes = rmp_serde::to_vec(&id).unwrap();
                bytes.extend_from_slice(&version_bytes);
                let state_key = prefixed_bytes(STATE_PREFIX, &bytes);
                db.get(state_key)
                    .unwrap()
                    .and_then(|state_bytes| rmp_serde::from_read(Cursor::new(state_bytes)).ok())
            })
        })
        .await
    }
}

#[async_trait::async_trait]
impl<T, B, I> StateProjectionWrite<T, I, B> for StateProjectionRocksDB
where
    T: EntitySnapshot + Send + Sync + Clone + Serialize + DeserializeOwned + 'static,
    T::Version: Send,
    B: Send + Sync + Clone + Serialize + DeserializeOwned + 'static,
    I: Identifier<For = T> + Send + Serialize + 'static,
{
    async fn write_predicted(&self, entity: Traced<Predicted<Bundled<T, B>>>) {}
    async fn write_confirmed(&self, entity: Traced<Confirmed<Bundled<T, B>>>) {}
    async fn remove(&self, id: I) -> Option<I> {
        None
    }
}

fn prefixed_key<T: Serialize>(prefix: &str, id: &T) -> Vec<u8> {
    let mut key_bytes = prefix.as_bytes().to_vec();
    let id_bytes = rmp_serde::to_vec(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

fn prefixed_bytes(prefix: &str, bytes: &[u8]) -> Vec<u8> {
    let mut key_bytes = prefix.as_bytes().to_vec();
    key_bytes.extend_from_slice(bytes);
    key_bytes
}
