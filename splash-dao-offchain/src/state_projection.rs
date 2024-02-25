use std::io::Cursor;
use std::sync::Arc;

use async_std::task::spawn_blocking;
use bloom_offchain::execution_engine::bundled::Bundled;
use serde::de::DeserializeOwned;
use serde::Serialize;
use spectrum_offchain::data::unique_entity::{AnyMod, Confirmed, Predicted, Traced};
use spectrum_offchain::data::{EntitySnapshot, HasIdentifier};
use spectrum_offchain::rocks::RocksConfig;

/// Projection of [T] state relative to the ledger.
#[async_trait::async_trait]
pub trait StateProjectionRead<T, B>
where
    T: EntitySnapshot + HasIdentifier,
    T::Id: Send + Serialize,
{
    async fn read(&self, id: T::Id) -> Option<AnyMod<Bundled<T, B>>>;
}

#[async_trait::async_trait]
pub trait StateProjectionWrite<T, B>
where
    T: EntitySnapshot + HasIdentifier,
    T::Id: Send,
{
    async fn write_predicted(&self, entity: Traced<Predicted<Bundled<T, B>>>);
    async fn write_confirmed(&self, entity: Traced<Confirmed<Bundled<T, B>>>);
    async fn remove(&self, id: T::Id) -> Option<T::Version>;
}

const LATEST_VERSION_PREFIX: &str = "id:";
const PREVIOUS_VERSION_PREFIX: &str = "p_id:";
const STATE_PREFIX: &str = "s:";

pub struct StateProjectionRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl StateProjectionRocksDB {
    pub fn new(conf: RocksConfig) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(conf.db_path).unwrap()),
        }
    }
}

#[async_trait::async_trait]
impl<T, B> StateProjectionRead<T, B> for StateProjectionRocksDB
where
    T: EntitySnapshot + HasIdentifier + Send + Sync + Clone + Serialize + DeserializeOwned + 'static,
    T::Version: Send,
    B: Send + Sync + Clone + Serialize + DeserializeOwned + 'static,
    T::Id: Send + Serialize + 'static,
{
    async fn read(&self, id: T::Id) -> Option<AnyMod<Bundled<T, B>>> {
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
impl<T, B> StateProjectionWrite<T, B> for StateProjectionRocksDB
where
    T: EntitySnapshot + HasIdentifier + Send + Sync + Clone + Serialize + DeserializeOwned + 'static,
    T::Version: Send + Serialize + DeserializeOwned,
    B: Send + Sync + Clone + Serialize + DeserializeOwned + 'static,
    T::Id: Send + Serialize + 'static,
{
    async fn write_predicted(&self, entity: Traced<Predicted<Bundled<T, B>>>) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let t = entity.state.0 .0.clone();
            let id = t.identifier();
            let tx = db.transaction();

            if let Some(prev_version) = entity.prev_state_id {
                let prev_version_key = prefixed_key(PREVIOUS_VERSION_PREFIX, &id);
                let new_prev_version_bytes = rmp_serde::to_vec_named(&prev_version).unwrap();
                tx.put(prev_version_key, new_prev_version_bytes).unwrap();
            }

            let current_version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
            let new_version_bytes = rmp_serde::to_vec_named(&t.version()).unwrap();

            let mut bytes = rmp_serde::to_vec(&id).unwrap();
            bytes.extend_from_slice(&new_version_bytes);
            let state_key = prefixed_bytes(STATE_PREFIX, &bytes);
            let state_bytes = rmp_serde::to_vec_named(&AnyMod::Predicted(entity)).unwrap();

            tx.put(state_key, state_bytes).unwrap();
            tx.put(current_version_key, new_version_bytes).unwrap();

            tx.commit().unwrap();
        })
        .await
    }

    async fn write_confirmed(&self, entity: Traced<Confirmed<Bundled<T, B>>>) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let t = entity.state.0 .0.clone();
            let id = t.identifier();
            let tx = db.transaction();

            if let Some(prev_version) = entity.prev_state_id {
                let prev_version_key = prefixed_key(PREVIOUS_VERSION_PREFIX, &id);
                let new_prev_version_bytes = rmp_serde::to_vec_named(&prev_version).unwrap();
                tx.put(prev_version_key, new_prev_version_bytes).unwrap();
            }

            let current_version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
            let new_version_bytes = rmp_serde::to_vec_named(&t.version()).unwrap();

            let mut bytes = rmp_serde::to_vec(&id).unwrap();
            bytes.extend_from_slice(&new_version_bytes);
            let state_key = prefixed_bytes(STATE_PREFIX, &bytes);
            let state_bytes = rmp_serde::to_vec_named(&AnyMod::Confirmed(entity)).unwrap();

            tx.put(state_key, state_bytes).unwrap();
            tx.put(current_version_key, new_version_bytes).unwrap();

            tx.commit().unwrap();
        })
        .await
    }

    async fn remove(&self, id: T::Id) -> Option<T::Version> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let current_version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
            let current_version_bytes = db.get(&current_version_key).unwrap().unwrap();
            let prev_version_key = prefixed_key(PREVIOUS_VERSION_PREFIX, &id);

            let tx = db.transaction();
            tx.delete(&current_version_key).unwrap();

            // Delete current state key
            {
                let mut bytes = rmp_serde::to_vec(&id).unwrap();
                bytes.extend_from_slice(&current_version_bytes);
                let old_current_state_key = prefixed_bytes(STATE_PREFIX, &bytes);
                tx.delete(old_current_state_key).unwrap();
            }

            if let Some(prev_version_bytes) = db.get(&prev_version_key).unwrap() {
                let prev_version: T::Version = rmp_serde::from_slice(&prev_version_bytes).unwrap();
                let mut bytes = rmp_serde::to_vec(&id).unwrap();
                bytes.extend_from_slice(&prev_version_bytes);
                let prev_state_key = prefixed_bytes(STATE_PREFIX, &bytes);
                let prev_state_bytes = db.get(prev_state_key).unwrap().unwrap();
                let prev_state: AnyMod<Bundled<T, B>> = rmp_serde::from_slice(&prev_state_bytes).unwrap();
                match prev_state {
                    AnyMod::Confirmed(Traced { prev_state_id, .. })
                    | AnyMod::Predicted(Traced { prev_state_id, .. }) => {
                        // Set new latest version
                        tx.put(current_version_key, prev_version_bytes).unwrap();
                        // Update new previous version if it exists
                        if let Some(prev_prev_version) = prev_state_id {
                            let prev_prev_version_bytes =
                                rmp_serde::to_vec_named(&prev_prev_version).unwrap();
                            tx.put(prev_version_key, prev_prev_version_bytes).unwrap();
                        }

                        tx.commit().unwrap();
                        Some(prev_version)
                    }
                    AnyMod::Unconfirmed(_) => unreachable!(),
                }
            } else {
                tx.commit().unwrap();
                None
            }
        })
        .await
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bloom_offchain::execution_engine::bundled::Bundled;
    use rand::RngCore;
    use serde::{Deserialize, Serialize};
    use spectrum_offchain::data::{
        unique_entity::{AnyMod, Confirmed, Predicted, Traced},
        EntitySnapshot, HasIdentifier, Identifier, Stable,
    };

    use crate::state_projection::{StateProjectionRead, StateProjectionWrite};

    use super::StateProjectionRocksDB;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
    struct Id(u32);

    impl Identifier for Id {
        type For = Entity;
    }

    #[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
    struct Entity {
        value: u32,
        id: u32,
        version: u32,
    }

    impl Entity {
        fn new(value: u32, id: u32) -> Self {
            Self {
                value,
                id,
                version: 0,
            }
        }

        fn incr_version(&mut self) {
            self.version += 1;
        }
    }

    impl Stable for Entity {
        type StableId = u32;

        fn stable_id(&self) -> Self::StableId {
            self.id
        }

        fn is_quasi_permanent(&self) -> bool {
            false
        }
    }

    impl HasIdentifier for Entity {
        type Id = Id;

        fn identifier(&self) -> Self::Id {
            Id(self.id)
        }
    }

    impl EntitySnapshot for Entity {
        type Version = u32;

        fn version(&self) -> Self::Version {
            self.version
        }
    }

    fn spawn_db() -> StateProjectionRocksDB {
        let rnd = rand::thread_rng().next_u32();
        let db = Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap());
        StateProjectionRocksDB { db }
    }

    #[tokio::test]
    async fn test_state_projection_rocksdb() {
        let sp = spawn_db();
        let mut entity = Entity::new(0, 0);
        let id = entity.identifier();
        sp.write_predicted(mk_predicted(entity.clone(), 0, None)).await;
        let e: AnyMod<Bundled<Entity, u32>> = sp.read(id).await.unwrap();
        assert!(matches!(e, AnyMod::Predicted(_)));

        let confirmed = mk_confirmed(entity.clone(), 1, None);
        sp.write_confirmed(confirmed.clone()).await;
        let e: AnyMod<Bundled<Entity, u32>> = sp.read(id).await.unwrap();
        if let AnyMod::Confirmed(Traced { state, prev_state_id }) = e {
            // This confirmed entity has same version as the previous predicted.
            assert_eq!(prev_state_id, None);
            assert_eq!(confirmed.state.0, state.0);
        } else {
            panic!("");
        }

        // Write successive entities with incremented versions

        let mut expected_entities = vec![];
        for _ in 0..10 {
            entity.incr_version();
            let confirmed = mk_confirmed(entity.clone(), entity.version(), Some(entity.version() - 1));
            expected_entities.push(confirmed.clone());
            sp.write_confirmed(confirmed.clone()).await;
            let e: AnyMod<Bundled<Entity, u32>> = sp.read(id).await.unwrap();
            if let AnyMod::Confirmed(Traced { state, prev_state_id }) = e {
                // This confirmed entity has same version as the previous predicted.
                assert_eq!(prev_state_id, Some(entity.version() - 1));
                assert_eq!(confirmed.state.0, state.0);
            } else {
                panic!("");
            }
        }

        // Start removing entities
        let mut expected_prev_version = entity.version() - 1;

        for expected_entity in expected_entities.into_iter().rev().skip(1) {
            let prev_ver =
                <StateProjectionRocksDB as StateProjectionWrite<Entity, u32>>::remove(&sp, id).await;
            assert_eq!(prev_ver, Some(expected_prev_version));
            let e: AnyMod<Bundled<Entity, u32>> = sp.read(id).await.unwrap();
            if let AnyMod::Confirmed(Traced { state, prev_state_id }) = e {
                assert_eq!(prev_state_id, Some(expected_prev_version - 1));
                assert_eq!(expected_entity.state.0, state.0);
            } else {
                panic!("");
            }

            expected_prev_version -= 1;
        }
    }

    fn mk_predicted(
        entity: Entity,
        bearer: u32,
        prev_state_id: Option<u32>,
    ) -> Traced<Predicted<Bundled<Entity, u32>>> {
        Traced::new(Predicted(Bundled(entity, bearer)), prev_state_id)
    }

    fn mk_confirmed(
        entity: Entity,
        bearer: u32,
        prev_state_id: Option<u32>,
    ) -> Traced<Confirmed<Bundled<Entity, u32>>> {
        Traced::new(Confirmed(Bundled(entity, bearer)), prev_state_id)
    }
}
