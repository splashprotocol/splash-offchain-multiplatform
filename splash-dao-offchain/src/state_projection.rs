use std::sync::Arc;

use async_std::task::spawn_blocking;
use bloom_offchain::execution_engine::bundled::Bundled;
use rocksdb::{Direction, IteratorMode, ReadOptions};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use spectrum_offchain::data::event::{AnyMod, Confirmed, Predicted, Traced};
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
    /// Returns Id of the entity whose latest version is given by `ver`, if it exists.
    async fn get_id(&self, ver: T::Version) -> Option<T::Id>;
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
/// Maps (PREVIOUS_VERSION_PREFIX | Id | VersionId) to the preceding version id.
const PREVIOUS_VERSION_PREFIX: &str = "p_id:";
const STATE_PREFIX: &str = "s:";

#[derive(Clone)]
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
    T::Id: Send + DeserializeOwned + Serialize + 'static,
{
    async fn read(&self, id: T::Id) -> Option<AnyMod<Bundled<T, B>>> {
        let db = self.db.clone();
        let version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
        spawn_blocking(move || {
            db.get(version_key).unwrap().and_then(|version_bytes| {
                let mut bytes = serde_json::to_vec(&id).unwrap();
                bytes.extend_from_slice(&version_bytes);
                let state_key = prefixed_bytes(STATE_PREFIX, &bytes);
                db.get(state_key)
                    .unwrap()
                    .and_then(|state_bytes| serde_json::from_slice(&state_bytes).ok())
            })
        })
        .await
    }

    async fn get_id(&self, ver: T::Version) -> Option<T::Id> {
        let db = Arc::clone(&self.db);
        let prefix = prefixed_key(PREVIOUS_VERSION_PREFIX, &ver);
        spawn_blocking(move || {
            let mut readopts = ReadOptions::default();
            readopts.set_iterate_range(rocksdb::PrefixRange(prefix.clone()));
            let mut iter = db.iterator_opt(IteratorMode::From(&prefix, Direction::Forward), readopts);
            if let Some(Ok((full_key, _))) = iter.next() {
                let id: T::Id = serde_json::from_slice(&full_key[prefix.len()..]).unwrap();
                Some(id)
            } else {
                None
            }
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
                let prev_ver_key = prev_version_key(&id, &t.version());
                let new_prev_version_bytes = serde_json::to_vec(&prev_version).unwrap();
                tx.put(prev_ver_key, new_prev_version_bytes).unwrap();
            }

            let current_version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
            let new_version_bytes = serde_json::to_vec(&t.version()).unwrap();

            let mut bytes = serde_json::to_vec(&id).unwrap();
            bytes.extend_from_slice(&new_version_bytes);
            let state_key = prefixed_bytes(STATE_PREFIX, &bytes);
            let predicted = AnyMod::Predicted(entity);
            println!("state_json: {:?}", serde_json::to_string_pretty(&predicted));
            let state_bytes = serde_json::to_vec(&predicted).unwrap();

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
                let prev_ver_key = prev_version_key(&id, &t.version());
                let new_prev_version_bytes = serde_json::to_vec(&prev_version).unwrap();
                tx.put(prev_ver_key, new_prev_version_bytes).unwrap();
            }

            let current_version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
            let new_version_bytes = serde_json::to_vec(&t.version()).unwrap();

            let mut bytes = serde_json::to_vec(&id).unwrap();
            bytes.extend_from_slice(&new_version_bytes);
            let state_key = prefixed_bytes(STATE_PREFIX, &bytes);
            let confirmed = AnyMod::Confirmed(entity);
            println!(
                "state_json: {}",
                serde_json::to_string_pretty(&confirmed).unwrap()
            );
            let state_bytes = serde_json::to_vec(&confirmed).unwrap();

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
            if let Some(current_version_bytes) = db.get(&current_version_key).unwrap() {
                let current_version: T::Version = serde_json::from_slice(&current_version_bytes).unwrap();
                let prev_ver_key = prev_version_key(&id, &current_version);

                let tx = db.transaction();
                tx.delete(&current_version_key).unwrap();

                // Delete current state key
                {
                    let mut bytes = serde_json::to_vec(&id).unwrap();
                    bytes.extend_from_slice(&current_version_bytes);
                    let old_current_state_key = prefixed_bytes(STATE_PREFIX, &bytes);
                    tx.delete(old_current_state_key).unwrap();
                }

                if let Some(prev_version_bytes) = db.get(&prev_ver_key).unwrap() {
                    let prev_version: T::Version = serde_json::from_slice(&prev_version_bytes).unwrap();
                    let mut bytes = serde_json::to_vec(&id).unwrap();
                    bytes.extend_from_slice(&prev_version_bytes);
                    let prev_state_key = prefixed_bytes(STATE_PREFIX, &bytes);
                    let prev_state_bytes = db.get(prev_state_key).unwrap().unwrap();
                    let prev_state: AnyMod<Bundled<T, B>> =
                        serde_json::from_slice(&prev_state_bytes).unwrap();
                    match prev_state {
                        AnyMod::Confirmed(Traced { prev_state_id, .. })
                        | AnyMod::Predicted(Traced { prev_state_id, .. }) => {
                            // Set new latest version
                            tx.put(current_version_key, prev_version_bytes).unwrap();
                            // Update new previous version if it exists
                            if let Some(prev_prev_version) = prev_state_id {
                                let prev_prev_version_bytes = serde_json::to_vec(&prev_prev_version).unwrap();
                                tx.put(prev_ver_key, prev_prev_version_bytes).unwrap();
                            }

                            tx.commit().unwrap();
                            Some(prev_version)
                        }
                    }
                } else {
                    tx.commit().unwrap();
                    None
                }
            } else {
                None
            }
        })
        .await
    }
}

fn prefixed_key<T: Serialize>(prefix: &str, id: &T) -> Vec<u8> {
    let mut key_bytes = prefix.as_bytes().to_vec();
    let id_bytes = serde_json::to_vec(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

fn prefixed_bytes(prefix: &str, bytes: &[u8]) -> Vec<u8> {
    let mut key_bytes = prefix.as_bytes().to_vec();
    key_bytes.extend_from_slice(bytes);
    key_bytes
}

fn prev_version_key<Id: Serialize, Ver: Serialize>(id: &Id, ver: &Ver) -> Vec<u8> {
    let mut key_bytes = PREVIOUS_VERSION_PREFIX.as_bytes().to_vec();
    let id_bytes = serde_json::to_vec(&id).unwrap();
    let ver_bytes = serde_json::to_vec(&ver).unwrap();
    // Encode version bytes first to allow efficient lookup of ID associated with version.
    key_bytes.extend_from_slice(&ver_bytes);
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bloom_offchain::execution_engine::bundled::Bundled;
    use cml_chain::transaction::TransactionOutput;
    use rand::RngCore;
    use serde::{Deserialize, Serialize};
    use spectrum_offchain::data::{
        event::{AnyMod, Confirmed, Predicted, Traced},
        EntitySnapshot, HasIdentifier, Identifier, Stable,
    };

    use crate::{
        entities::onchain::inflation_box::InflationBoxSnapshot,
        state_projection::{StateProjectionRead, StateProjectionWrite},
    };

    use super::StateProjectionRocksDB;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
            assert_eq!(
                Some(expected_entity.state.0 .0.identifier()),
                <StateProjectionRocksDB as StateProjectionRead<Entity, u32>>::get_id(
                    &sp,
                    expected_entity.state.version()
                )
                .await
            );
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

    #[test]
    fn deserialize_inflation_box() {
        let s = r#"
        {
  "Confirmed": {
    "state": [
      [
        {
          "last_processed_epoch": 0,
          "splash_reserves": [
            32000000000000,
            null
          ],
          "script_hash": "590b543a6bb643b4bd5d0c173e9f56816b60d1f79aeb0473ff24c66d"
        },
        [
          "e54d54359cd0da7b5ee892c3c83b3f108894d4ef76bde10df66f87c429600e88",
          0
        ]
      ],
      {
        "ConwayFormatTxOut": {
          "address": "addr_test1wpvsk4p6dwmy8d9at5xpw05l26qkkcx377dwkprnlujvvmglugdhw",
          "amount": {
            "coin": 3000000,
            "multiasset": {
              "adf2425c138138efce80fd0b2ed8f227caf052f9ec44b8a92e942dfa": {
                "53504c415348": 32000000000000
              }
            }
          },
          "datum_option": {
            "Datum": {
              "datum": {
                "int": 0
              }
            }
          },
          "script_reference": null
        }
      }
    ],
    "prev_state_id": null
  }
}
        "#;
        let confirmed: AnyMod<Confirmed<Bundled<InflationBoxSnapshot, TransactionOutput>>> =
            serde_json::from_str(s).unwrap();
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
