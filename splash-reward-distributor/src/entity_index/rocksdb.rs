use std::hash::Hash;
use std::path::Path;
use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use bloom_offchain::execution_engine::bundled::Bundled;
use const_format::formatcp;
use log::trace;
use rocksdb::{Options, TransactionDB, TransactionDBOptions};
use serde::Deserialize;
use serde::{de::DeserializeOwned, Serialize};
use spectrum_offchain::domain::Stable;
use spectrum_offchain::domain::{
    event::{AnyMod, Confirmed, Predicted, Traced},
    EntitySnapshot,
};
use tokio::task::spawn_blocking;

use crate::entity_index::{HarvestOrderIndex, HarvestOrderStatus, Mod};
use splash_yf_offchain::entities::{
    auth_manager::AuthManager, buffer_wallet::BufferWallet, gauge::Gauge, harvest_order::HarvestOrder,
};

#[async_trait::async_trait]
pub trait OnChainIndex<Bearer>
where
    Bearer: Serialize + DeserializeOwned,
{
    async fn read<T>(&self, id: T::StableId) -> Option<AnyMod<Bundled<T, Bearer>>>
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send + Serialize + DeserializeOwned + 'static,
        T::StableId: Serialize + DeserializeOwned + 'static;

    async fn write_predicted<T>(&self, entity: Traced<Predicted<Bundled<T, Bearer>>>)
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send + Clone + Serialize + 'static,
        T::StableId: Serialize + DeserializeOwned + 'static;

    async fn write_confirmed<T>(&self, entity: Traced<Confirmed<Bundled<T, Bearer>>>)
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send + Clone + Serialize + 'static,
        T::StableId: Serialize + DeserializeOwned + 'static;

    /// Deletes latest version of the entity and returns the previous version if it exists.
    async fn remove<T>(&self, id: T::StableId, version: T::Version) -> Option<T::Version>
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send + Clone + Serialize + DeserializeOwned + 'static,
        T::StableId: Serialize + DeserializeOwned + 'static,
        T::Version: Debug + Eq + PartialEq;
}

#[derive(Clone)]
pub struct IndexerDB {
    pub db: Arc<TransactionDB>,
}

const LATEST_VERSION_PREFIX: &str = "id:";
/// Maps (PREVIOUS_VERSION_PREFIX | Id | VersionId) to the preceding version id.
const PREVIOUS_VERSION_PREFIX: &str = "p_id:";
const STATE_PREFIX: &str = "s:";

impl IndexerDB {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Self {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db_opts = TransactionDBOptions::default();
        Self {
            db: Arc::new(TransactionDB::open_cf(&opts, &db_opts, db_path, COLUMN_FAMILIES).unwrap()),
        }
    }
}

#[async_trait::async_trait]
impl<Bearer> OnChainIndex<Bearer> for IndexerDB
where
    Bearer: Send + 'static + Serialize + DeserializeOwned,
{
    async fn read<T>(&self, id: T::StableId) -> Option<AnyMod<Bundled<T, Bearer>>>
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send + Serialize + DeserializeOwned + 'static,
        T::StableId: Serialize + DeserializeOwned + 'static,
    {
        let db = self.db.clone();
        let version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
        spawn_blocking(move || {
            let cf = db.cf_handle(T::ID).unwrap();
            db.get_cf(cf, version_key).unwrap().and_then(|version_bytes| {
                let mut bytes = rmp_serde::to_vec(&id).unwrap();
                bytes.extend_from_slice(&version_bytes);
                let state_key = prefixed_bytes(STATE_PREFIX, &bytes);
                db.get_cf(cf, state_key)
                    .unwrap()
                    .and_then(|state_bytes| rmp_serde::from_slice(&state_bytes).ok())
            })
        })
        .await
        .unwrap()
    }

    async fn write_predicted<T>(&self, entity: Traced<Predicted<Bundled<T, Bearer>>>)
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send + Clone + Serialize + 'static,
        T::StableId: Serialize + DeserializeOwned + 'static,
    {
        let db = self.db.clone();
        spawn_blocking(move || {
            let t = entity.state.0 .0.clone();
            let id = t.stable_id();
            let tx = db.transaction();
            let cf = db.cf_handle(T::ID).unwrap();

            if let Some(prev_version) = entity.prev_state_id {
                let prev_ver_key = prev_version_key(&id, &t.version());
                let new_prev_version_bytes = rmp_serde::to_vec(&prev_version).unwrap();
                tx.put_cf(cf, prev_ver_key, new_prev_version_bytes).unwrap();
            }

            let current_version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
            let new_version_bytes = rmp_serde::to_vec(&t.version()).unwrap();

            let mut bytes = rmp_serde::to_vec(&id).unwrap();
            bytes.extend_from_slice(&new_version_bytes);
            let state_key = prefixed_bytes(STATE_PREFIX, &bytes);
            let predicted = AnyMod::Predicted(entity);
            trace!(
                "write_predicte() id: {}, version: {}, state_json: {}",
                id,
                t.version(),
                serde_json::to_string_pretty(&predicted).unwrap()
            );
            let state_bytes = rmp_serde::to_vec_named(&predicted).unwrap();

            tx.put_cf(cf, state_key, state_bytes).unwrap();
            tx.put_cf(cf, current_version_key, new_version_bytes).unwrap();

            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn write_confirmed<T>(&self, entity: Traced<Confirmed<Bundled<T, Bearer>>>)
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send + Clone + Serialize + 'static,
        T::StableId: Serialize + DeserializeOwned + 'static,
    {
        let db = self.db.clone();
        spawn_blocking(move || {
            let t = entity.state.0 .0.clone();
            let id = t.stable_id();
            let tx = db.transaction();
            let cf = db.cf_handle(T::ID).unwrap();

            if let Some(prev_version) = entity.prev_state_id {
                let prev_ver_key = prev_version_key(&id, &t.version());
                let new_prev_version_bytes = rmp_serde::to_vec(&prev_version).unwrap();
                tx.put_cf(cf, prev_ver_key, new_prev_version_bytes).unwrap();
            }

            let current_version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
            let new_version_bytes = rmp_serde::to_vec(&t.version()).unwrap();

            let mut bytes = rmp_serde::to_vec(&id).unwrap();
            bytes.extend_from_slice(&new_version_bytes);
            let state_key = prefixed_bytes(STATE_PREFIX, &bytes);
            let confirmed = AnyMod::Confirmed(entity);
            trace!(
                "write_confirmed() id: {}, version: {}, state_json: {}",
                id,
                t.version(),
                serde_json::to_string_pretty(&confirmed).unwrap()
            );
            let state_bytes = rmp_serde::to_vec_named(&confirmed).unwrap();

            tx.put_cf(cf, state_key, state_bytes).unwrap();
            tx.put_cf(cf, current_version_key, new_version_bytes).unwrap();

            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    /// Deletes latest version(StateId) of the entity and returns the previous version if it exists.
    async fn remove<T>(&self, id: T::StableId, version: T::Version) -> Option<T::Version>
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send + Clone + Serialize + DeserializeOwned + 'static,
        T::StableId: Serialize + DeserializeOwned + 'static,
        T::Version: Debug + Eq + PartialEq,
    {
        let db = self.db.clone();
        spawn_blocking(move || {
            let current_version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
            let cf = db.cf_handle(T::ID).unwrap();
            if let Some(current_version_bytes) = db.get_cf(cf, &current_version_key).unwrap() {
                let current_version: T::Version = rmp_serde::from_slice(&current_version_bytes).unwrap();
                assert_eq!(current_version, version);
                trace!(
                    "StateProjectionWrite::remove: id: {}, ver: {}",
                    id,
                    current_version
                );
                let prev_ver_key = prev_version_key(&id, &current_version);

                let tx = db.transaction();
                tx.delete_cf(cf, &current_version_key).unwrap();

                // Delete current state key
                {
                    let mut bytes = rmp_serde::to_vec(&id).unwrap();
                    bytes.extend_from_slice(&current_version_bytes);
                    let old_current_state_key = prefixed_bytes(STATE_PREFIX, &bytes);
                    tx.delete_cf(cf, old_current_state_key).unwrap();
                }

                if let Some(prev_version_bytes) = db.get_cf(cf, &prev_ver_key).unwrap() {
                    let prev_version: T::Version = rmp_serde::from_slice(&prev_version_bytes).unwrap();
                    trace!("StateProjectionWrite::remove: prev_version {}", prev_version);
                    let mut bytes = rmp_serde::to_vec(&id).unwrap();
                    bytes.extend_from_slice(&prev_version_bytes);
                    let prev_state_key = prefixed_bytes(STATE_PREFIX, &bytes);
                    let prev_state_bytes = db.get_cf(cf, prev_state_key).unwrap().unwrap();
                    let prev_state: AnyMod<Bundled<T, Bearer>> =
                        rmp_serde::from_slice(&prev_state_bytes).unwrap();
                    match prev_state {
                        AnyMod::Confirmed(Traced { prev_state_id, .. })
                        | AnyMod::Predicted(Traced { prev_state_id, .. }) => {
                            // Set new latest version
                            tx.put_cf(cf, current_version_key, prev_version_bytes).unwrap();
                            // Update new previous version if it exists
                            if let Some(prev_prev_version) = prev_state_id {
                                let prev_prev_version_bytes = rmp_serde::to_vec(&prev_prev_version).unwrap();
                                tx.put_cf(cf, prev_ver_key, prev_prev_version_bytes).unwrap();
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
        .unwrap()
    }
}

#[async_trait::async_trait]
impl<StateId, Bearer> HarvestOrderIndex<StateId, Bearer> for IndexerDB
where
    StateId: Copy + Eq + Hash + Send + Sync + Debug + Display + Serialize + DeserializeOwned + 'static,
    Bearer: Send + Serialize + DeserializeOwned + 'static,
{
    async fn read_harvest_order(
        &self,
        id: StateId,
    ) -> Option<Mod<Bundled<(HarvestOrder<StateId>, HarvestOrderStatus), Bearer>>> {
        let wrapped = self.read::<HarvestOrderWrap<StateId>>(id).await;

        wrapped.map(|h| match h {
            AnyMod::Confirmed(t) => {
                let harvest_order = t.state.0 .0.order;
                let status = t.state.0 .0.status;
                let bearer = t.state.0 .1;
                Mod::Confirmed(Bundled((harvest_order, status), bearer))
            }
            AnyMod::Predicted(t) => {
                let harvest_order = t.state.0 .0.order;
                let status = t.state.0 .0.status;
                let bearer = t.state.0 .1;
                Mod::Predicted(Bundled((harvest_order, status), bearer))
            }
        })
    }

    async fn write_predicted_spend_harvest_order(&self, id: StateId) {
        if let Some(AnyMod::Confirmed(Traced {
            prev_state_id,
            state: Confirmed(Bundled(mut harvest_order, bearer)),
        })) = self.read::<HarvestOrderWrap<StateId>>(id).await
        {
            assert!(matches!(harvest_order.status, HarvestOrderStatus::Unspent));
            harvest_order.status = HarvestOrderStatus::Spent;

            let state: Predicted<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                Predicted(Bundled(harvest_order, bearer));
            self.write_predicted(Traced { state, prev_state_id }).await;
        }
    }

    async fn write_confirmed_spend_harvest_order(&self, id: StateId) {
        if let Some(any_mod) = self.read::<HarvestOrderWrap<StateId>>(id).await {
            match any_mod {
                AnyMod::Confirmed(Traced {
                    prev_state_id,
                    state: Confirmed(Bundled(mut harvest_order, bearer)),
                }) => {
                    assert!(matches!(harvest_order.status, HarvestOrderStatus::Unspent));
                    harvest_order.status = HarvestOrderStatus::Spent;

                    let state: Confirmed<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                        Confirmed(Bundled(harvest_order, bearer));
                    self.write_confirmed(Traced { state, prev_state_id }).await;
                }
                AnyMod::Predicted(Traced {
                    prev_state_id,
                    state: Predicted(Bundled(mut harvest_order, bearer)),
                }) => {
                    assert!(matches!(harvest_order.status, HarvestOrderStatus::Spent));
                    harvest_order.status = HarvestOrderStatus::Spent;

                    let state: Confirmed<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                        Confirmed(Bundled(harvest_order, bearer));
                    self.write_confirmed(Traced { state, prev_state_id }).await;
                }
            }
        }
    }

    async fn write_confirmed_refund_harvest_order(&self, id: StateId) {
        if let Some(any_mod) = self.read::<HarvestOrderWrap<StateId>>(id).await {
            match any_mod {
                AnyMod::Confirmed(Traced {
                    prev_state_id,
                    state: Confirmed(Bundled(mut harvest_order, bearer)),
                }) => {
                    harvest_order.status = HarvestOrderStatus::Refunded;
                    assert!(prev_state_id.is_none());

                    let state: Confirmed<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                        Confirmed(Bundled(harvest_order, bearer));
                    self.write_confirmed(Traced { state, prev_state_id }).await;
                }
                AnyMod::Predicted(Traced {
                    prev_state_id,
                    state: Predicted(Bundled(mut harvest_order, bearer)),
                }) => {
                    harvest_order.status = HarvestOrderStatus::Refunded;
                    assert!(prev_state_id.is_none());

                    let state: Confirmed<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                        Confirmed(Bundled(harvest_order, bearer));
                    self.write_confirmed(Traced { state, prev_state_id }).await;
                }
            }
        }
    }

    async fn write_confirmed_harvest_order(&self, order: Confirmed<Bundled<HarvestOrder<StateId>, Bearer>>) {
        let id = order.0 .0.id;
        assert!(
            <IndexerDB as OnChainIndex<Bearer>>::read::<HarvestOrderWrap<StateId>>(self, id)
                .await
                .is_none()
        );
        let harvest_order = HarvestOrderWrap {
            order: order.0 .0,
            status: HarvestOrderStatus::Unspent,
        };
        let bearer = order.0 .1;
        let state = Confirmed(Bundled(harvest_order, bearer));
        self.write_confirmed(Traced {
            state,
            prev_state_id: None,
        })
        .await;
    }

    async fn unconsume_harvest_order(&self, id: StateId) {
        if let Some(any_mod) = self.read::<HarvestOrderWrap<StateId>>(id).await {
            match any_mod {
                AnyMod::Confirmed(Traced {
                    prev_state_id,
                    state: Confirmed(Bundled(mut harvest_order, bearer)),
                }) => {
                    assert!(matches!(
                        harvest_order.status,
                        HarvestOrderStatus::Spent | HarvestOrderStatus::Refunded
                    ));
                    harvest_order.status = HarvestOrderStatus::Unspent;
                    assert!(prev_state_id.is_none());

                    let state: Confirmed<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                        Confirmed(Bundled(harvest_order, bearer));
                    self.write_confirmed(Traced { state, prev_state_id }).await;
                }
                AnyMod::Predicted(Traced {
                    prev_state_id,
                    state: Predicted(Bundled(mut harvest_order, bearer)),
                }) => {
                    assert!(matches!(harvest_order.status, HarvestOrderStatus::Spent));
                    harvest_order.status = HarvestOrderStatus::Unspent;
                    assert!(prev_state_id.is_none());

                    let state: Confirmed<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                        Confirmed(Bundled(harvest_order, bearer));
                    self.write_confirmed(Traced { state, prev_state_id }).await;
                }
            }
        }
    }

    async fn remove_created_harvest_order(&self, id: StateId) {
        let r = <IndexerDB as OnChainIndex<Bearer>>::read::<HarvestOrderWrap<StateId>>(self, id).await;
        assert!(matches!(
            r,
            Some(AnyMod::Confirmed(Traced {
                state: Confirmed(Bundled(
                    HarvestOrderWrap {
                        status: HarvestOrderStatus::Unspent,
                        ..
                    },
                    _
                )),
                ..
            }))
        ));
        assert!(
            <IndexerDB as OnChainIndex<Bearer>>::remove::<HarvestOrderWrap<StateId>>(self, id, id)
                .await
                .is_none()
        );
    }
}

#[derive(Clone, Serialize, Deserialize)]
/// This wrapper type exists to allow `HarvestOrder`s to be treated as an `EntitySnapshot`. This
/// simplifies the implementation of IndexerDB, as we'd otherwise need custom logic just for
/// `HarvestOrder`.
pub struct HarvestOrderWrap<StateId> {
    order: HarvestOrder<StateId>,
    status: HarvestOrderStatus,
}

impl<StateId> Stable for HarvestOrderWrap<StateId>
where
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
{
    type StableId = StateId;

    fn stable_id(&self) -> Self::StableId {
        self.order.id
    }

    fn is_quasi_permanent(&self) -> bool {
        false
    }
}

impl<StateId> EntitySnapshot for HarvestOrderWrap<StateId>
where
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
{
    type Version = StateId;

    fn version(&self) -> Self::Version {
        self.order.id
    }
}

mod unique_ids {
    // Sealed trait
    pub trait UniqueId {
        const ID: &str;
    }
}

impl<StateId> unique_ids::UniqueId for HarvestOrderWrap<StateId> {
    const ID: &str = formatcp!("CF_{}", EntityId::HarvestOrder as u8);
}

impl<StateId> unique_ids::UniqueId for BufferWallet<StateId> {
    const ID: &str = formatcp!("CF_{}", EntityId::BufferWallet as u8);
}

impl<GaugeId, StateId> unique_ids::UniqueId for Gauge<GaugeId, StateId> {
    const ID: &str = formatcp!("CF_{}", EntityId::Gauge as u8);
}

impl<GaugeId, StateId> unique_ids::UniqueId for AuthManager<GaugeId, StateId> {
    const ID: &str = formatcp!("CF_{}", EntityId::AuthManager as u8);
}

pub(crate) const COLUMN_FAMILIES: [&str; 4] = [
    <HarvestOrderWrap<u8> as unique_ids::UniqueId>::ID,
    <BufferWallet<u8> as unique_ids::UniqueId>::ID,
    <Gauge<u8, u8> as unique_ids::UniqueId>::ID,
    <AuthManager<u8, u8> as unique_ids::UniqueId>::ID,
];

#[repr(u8)]
#[derive(Eq, PartialEq)]
enum EntityId {
    HarvestOrder = 0,
    BufferWallet = 1,
    Gauge = 2,
    AuthManager = 3,
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

fn prev_version_key<Id: Serialize, Ver: Serialize>(id: &Id, ver: &Ver) -> Vec<u8> {
    let mut key_bytes = PREVIOUS_VERSION_PREFIX.as_bytes().to_vec();
    let id_bytes = rmp_serde::to_vec(&id).unwrap();
    let ver_bytes = rmp_serde::to_vec(&ver).unwrap();
    // Encode version bytes first to allow efficient lookup of ID associated with version.
    key_bytes.extend_from_slice(&ver_bytes);
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

#[cfg(test)]
mod tests {

    use bloom_offchain::execution_engine::bundled::Bundled;
    use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
    use rand::{Rng, RngCore};
    use spectrum_cardano_lib::address::{PlutusAddress, PlutusCredential};
    use spectrum_offchain::domain::{
        event::{AnyMod, Confirmed, Predicted, Traced},
        EntitySnapshot, Stable,
    };
    use splash_dao_offchain::routines::Slot;

    use crate::entity_index::rocksdb::{HarvestOrderIndex, HarvestOrderStatus, IndexerDB, Mod, OnChainIndex};
    use splash_yf_offchain::entities::{
        buffer_wallet::BufferWallet, gauge::Gauge, harvest_order::HarvestOrder,
    };

    #[tokio::test]
    async fn test_state_harvest_orders() {
        let db = spawn_db();
        let mut orders = vec![];
        let n = 20;
        for i in 0..n {
            let h = confirmed(mk_harvest_order(i), i);
            orders.push(h.0.clone());
            db.write_confirmed_harvest_order(h).await;
        }

        for i in 0..n {
            let p: Mod<Bundled<(HarvestOrder<u32>, HarvestOrderStatus), _>> =
                db.read_harvest_order(i).await.unwrap();
            let Bundled(order, bearer) = orders[i as usize].clone();
            let expected = Mod::Confirmed(Bundled((order, HarvestOrderStatus::Unspent), bearer));
            assert_eq!(expected, p);
        }

        // Spend
        for h in &orders {
            let id = h.1;
            <IndexerDB as HarvestOrderIndex<u32, u32>>::write_predicted_spend_harvest_order(&db, id).await;
            let p: Mod<Bundled<(HarvestOrder<u32>, HarvestOrderStatus), u32>> =
                db.read_harvest_order(id).await.unwrap();
            let Mod::Predicted(Bundled((order, status), bearer)) = p else {
                panic!()
            };
            assert_eq!((order, status), (h.0.clone(), HarvestOrderStatus::Spent));
            assert_eq!(h.1, bearer);
        }

        let unconsume_orders = || async {
            for i in 0..n {
                <IndexerDB as HarvestOrderIndex<u32, u32>>::unconsume_harvest_order(&db, i).await;
                let p: Mod<Bundled<(HarvestOrder<u32>, HarvestOrderStatus), _>> =
                    db.read_harvest_order(i).await.unwrap();
                let Bundled(order, bearer) = orders[i as usize].clone();
                let expected = Mod::Confirmed(Bundled((order, HarvestOrderStatus::Unspent), bearer));
                assert_eq!(expected, p);
            }
        };

        // Undo the spending
        unconsume_orders().await;

        // Refund
        for i in 0..n {
            <IndexerDB as HarvestOrderIndex<u32, u32>>::write_confirmed_refund_harvest_order(&db, i).await;
            let p: Mod<Bundled<(HarvestOrder<u32>, HarvestOrderStatus), _>> =
                db.read_harvest_order(i).await.unwrap();
            let Bundled(order, bearer) = orders[i as usize].clone();
            let expected = Mod::Confirmed(Bundled((order, HarvestOrderStatus::Refunded), bearer));
            assert_eq!(expected, p);
        }

        // Undo the refunds
        unconsume_orders().await;

        // Remove the orders
        for i in 0..n {
            <IndexerDB as HarvestOrderIndex<u32, u32>>::remove_created_harvest_order(&db, i).await;
            let p: Option<Mod<Bundled<(HarvestOrder<u32>, HarvestOrderStatus), u32>>> =
                db.read_harvest_order(i).await;
            assert!(p.is_none());
        }
    }

    #[tokio::test]
    async fn test_on_chain_index_buffer_wallet() {
        let db = spawn_db();
        let buffer_wallet = mk_buffer_wallet();
        let id = buffer_wallet.stable_id();
        let traced_wallet = mk_traced_predicted(buffer_wallet, 0, None);
        let mut wallet = traced_wallet.state.0.clone();
        db.write_predicted(traced_wallet.clone()).await;
        let e: AnyMod<Bundled<BufferWallet<u32>, u32>> = db.read(id).await.unwrap();
        assert!(matches!(e, AnyMod::Predicted(_)));
        assert_eq!(e.erased(), traced_wallet.state.0);

        let mut expected_entities = vec![];
        for _ in 0..10 {
            let prev_version = wallet.0.state_id;
            wallet.0.state_id += 1;
            let confirmed = mk_traced_confirmed(wallet.0.clone(), wallet.version(), Some(prev_version));
            expected_entities.push(confirmed.clone());
            db.write_confirmed(confirmed.clone()).await;
            let e: AnyMod<Bundled<BufferWallet<u32>, u32>> = db.read(id).await.unwrap();
            if let AnyMod::Confirmed(Traced { state, prev_state_id }) = e {
                // This confirmed entity has same version as the previous predicted.
                assert_eq!(prev_state_id, Some(prev_version));
                assert_eq!(confirmed.state.0, state.0);
            } else {
                panic!("");
            }
        }

        // Start removing entities
        let mut expected_prev_version = wallet.version() - 1;

        for expected_entity in expected_entities.into_iter().rev().skip(1) {
            assert_eq!(Some(expected_entity.state.0 .0.stable_id()), Some(id));
            dbg!(expected_entity.state.version());
            let prev_ver = <IndexerDB as OnChainIndex<u32>>::remove::<BufferWallet<u32>>(
                &db,
                id,
                expected_entity.state.version() + 1,
            )
            .await
            .unwrap();
            assert_eq!(prev_ver, expected_prev_version);
            let e: AnyMod<Bundled<BufferWallet<u32>, u32>> = db.read(id).await.unwrap();
            if let AnyMod::Confirmed(Traced { state, prev_state_id }) = e {
                assert_eq!(prev_state_id, Some(expected_prev_version - 1));
                assert_eq!(expected_entity.state.0, state.0);
            } else {
                panic!("");
            }

            expected_prev_version -= 1;
        }
    }

    #[tokio::test]
    async fn test_on_chain_index_gauge() {
        let db = spawn_db();
        let gauge = mk_gauge();
        let id = gauge.stable_id();
        let traced_wallet = mk_traced_predicted(gauge, 0, None);
        let mut wallet = traced_wallet.state.0.clone();
        db.write_predicted(traced_wallet.clone()).await;
        let e: AnyMod<Bundled<Gauge<u32, u32>, u32>> = db.read(id).await.unwrap();
        assert!(matches!(e, AnyMod::Predicted(_)));
        assert_eq!(e.erased(), traced_wallet.state.0);

        let mut expected_entities = vec![];
        for _ in 0..10 {
            let prev_version = wallet.0.state_id;
            wallet.0.state_id += 1;
            let confirmed = mk_traced_confirmed(wallet.0.clone(), wallet.version(), Some(prev_version));
            expected_entities.push(confirmed.clone());
            db.write_confirmed(confirmed.clone()).await;
            let e: AnyMod<Bundled<Gauge<u32, u32>, u32>> = db.read(id).await.unwrap();
            if let AnyMod::Confirmed(Traced { state, prev_state_id }) = e {
                // This confirmed entity has same version as the previous predicted.
                assert_eq!(prev_state_id, Some(prev_version));
                assert_eq!(confirmed.state.0, state.0);
            } else {
                panic!("");
            }
        }

        // Start removing entities
        let mut expected_prev_version = wallet.version() - 1;

        for expected_entity in expected_entities.into_iter().rev().skip(1) {
            assert_eq!(Some(expected_entity.state.0 .0.stable_id()), Some(id));
            dbg!(expected_entity.state.version());
            let prev_ver = <IndexerDB as OnChainIndex<u32>>::remove::<Gauge<u32, u32>>(
                &db,
                id,
                expected_entity.state.version() + 1,
            )
            .await
            .unwrap();
            assert_eq!(prev_ver, expected_prev_version);
            let e: AnyMod<Bundled<Gauge<u32, u32>, u32>> = db.read(id).await.unwrap();
            if let AnyMod::Confirmed(Traced { state, prev_state_id }) = e {
                assert_eq!(prev_state_id, Some(expected_prev_version - 1));
                assert_eq!(expected_entity.state.0, state.0);
            } else {
                panic!("");
            }

            expected_prev_version -= 1;
        }
    }

    fn mk_harvest_order(id: u32) -> HarvestOrder<u32> {
        let mut rng = rand::thread_rng();
        let mut array = [0u8; 28];
        rng.fill(&mut array);
        let account_key = Ed25519KeyHash::from_raw_bytes(&array).unwrap();
        let reward_receiver = PlutusAddress {
            payment_cred: PlutusCredential::PubKey(account_key),
            stake_cred: None,
        };
        HarvestOrder {
            id,
            account_key,
            issued_at: Slot(100),
            reward_receiver,
        }
    }

    fn mk_gauge() -> Gauge<u32, u32> {
        let mut rng = rand::thread_rng();
        Gauge {
            id: rng.next_u32(),
            state_id: rng.next_u32(),
            balance: 1000,
        }
    }

    fn mk_buffer_wallet() -> BufferWallet<u32> {
        let mut rng = rand::thread_rng();
        BufferWallet {
            state_id: rng.next_u32(),
            balance: 1_000_000,
        }
    }

    fn mk_traced_predicted<T>(
        entity: T,
        bearer: u32,
        prev_state_id: Option<u32>,
    ) -> Traced<Predicted<Bundled<T, u32>>>
    where
        T: EntitySnapshot<Version = u32>,
    {
        Traced::new(Predicted(Bundled(entity, bearer)), prev_state_id)
    }

    fn mk_traced_confirmed<T>(
        entity: T,
        bearer: u32,
        prev_state_id: Option<u32>,
    ) -> Traced<Confirmed<Bundled<T, u32>>>
    where
        T: EntitySnapshot<Version = u32>,
    {
        Traced::new(Confirmed(Bundled(entity, bearer)), prev_state_id)
    }

    fn spawn_db() -> IndexerDB {
        let rnd = rand::thread_rng().next_u32();
        let db_path = format!("./tmp/{}", rnd);
        IndexerDB::new(db_path)
    }

    fn predicted<T>(t: T, bearer: u32) -> Predicted<Bundled<T, u32>> {
        Predicted(Bundled(t, bearer))
    }

    fn confirmed<T>(t: T, bearer: u32) -> Confirmed<Bundled<T, u32>> {
        Confirmed(Bundled(t, bearer))
    }
}
