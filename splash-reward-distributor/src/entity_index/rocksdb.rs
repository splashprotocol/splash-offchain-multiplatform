use std::hash::Hash;
use std::path::Path;
use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use bloom_offchain::execution_engine::bundled::Bundled;
use cml_crypto::Ed25519KeyHash;
use cml_crypto::RawBytesEncoding;
use const_format::formatcp;
use log::trace;
use rocksdb::{ColumnFamily, Options, Transaction, TransactionDB, TransactionDBOptions};
use rs_merkle::MerkleTree;
use serde::Deserialize;
use serde::{de::DeserializeOwned, Serialize};
use spectrum_offchain::data::circular_buffer_rocksdb::{
    buffer_init, buffer_pop_back, buffer_push_back, buffer_read_back,
};
use spectrum_offchain::domain::Stable;
use spectrum_offchain::domain::{
    event::{AnyMod, Confirmed, Predicted, Traced},
    EntitySnapshot,
};
use splash_dao_offchain::routines::Slot;
use splash_yf_offchain::entities::buffer_wallet::BufferWalletWrap;
use splash_yf_offchain::Epoch;
use tokio::task::spawn_blocking;

use crate::entity_index::rocksdb::unique_ids::UniqueId;
use crate::entity_index::{HarvestOrderIndex, HarvestOrderSpend, HarvestOrderStatus, IndexedMerkleTree, Mod};
use splash_yf_offchain::entities::{auth_manager::AuthManager, gauge::Gauge, harvest_order::HarvestOrder};

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
    max_number_merkle_tree_snapshots: u8,
}

const LATEST_VERSION_PREFIX: &str = "id:";
/// Maps (PREVIOUS_VERSION_PREFIX | Id | VersionId) to the preceding version id.
const PREVIOUS_VERSION_PREFIX: &str = "p_id:";
const STATE_PREFIX: &str = "s:";

impl IndexerDB {
    pub fn new<P: AsRef<Path>>(db_path: P, max_number_merkle_tree_snapshots: u8) -> Self {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db_opts = TransactionDBOptions::default();
        let db: Arc<TransactionDB> =
            Arc::new(TransactionDB::open_cf(&opts, &db_opts, db_path, COLUMN_FAMILIES).unwrap());

        let merkle_snapshot_cf = db.cf_handle(CF_MERKLE_TREE_SNAPSHOTS).unwrap();
        let new_store = buffer_init::<u8>(&db, merkle_snapshot_cf);
        let indexed_tree = IndexedMerkleTree {
            tree: MerkleTree::new(),
            slot: Slot(0), // TODO: fix with deployment (DEX-935)
        };
        if new_store {
            let tx = db.transaction();
            buffer_push_back::<u8, IndexedMerkleTree>(
                &indexed_tree,
                max_number_merkle_tree_snapshots,
                &tx,
                merkle_snapshot_cf,
            );
            tx.commit().unwrap();
        }
        Self {
            db,
            max_number_merkle_tree_snapshots,
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
        spawn_blocking(move || {
            let cf = db.cf_handle(T::ID).unwrap();
            let tx = db.transaction();
            read_inner(id, &tx, cf)
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
            let tx = db.transaction();
            let cf = db.cf_handle(T::ID).unwrap();

            write_confirmed_inner(entity, &tx, cf);

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

    async fn write_predicted_spend_harvest_order(&self, id: StateId, predicted_spend: &HarvestOrderSpend) {
        if let Some(AnyMod::Confirmed(Traced {
            prev_state_id,
            state: Confirmed(Bundled(mut harvest_order, bearer)),
        })) = self.read::<HarvestOrderWrap<StateId>>(id).await
        {
            assert!(matches!(harvest_order.status, HarvestOrderStatus::Unspent));

            let current_epoch = predicted_spend.epoch_end.next();
            harvest_order.status = HarvestOrderStatus::Spent(current_epoch);

            let state: Predicted<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                Predicted(Bundled(harvest_order, bearer));
            self.write_predicted(Traced { state, prev_state_id }).await;
        }
    }

    async fn write_confirmed_spend_harvest_orders(
        &self,
        orders: Vec<(StateId, HarvestOrderSpend)>,
        confirmed_slot: u64,
    ) {
        let db = self.db.clone();
        let max_number_merkle_tree_snapshots = self.max_number_merkle_tree_snapshots;
        spawn_blocking(move || {
            let tx = db.transaction();
            let cf = db.cf_handle(HarvestOrderWrap::<StateId>::ID).unwrap();
            let mut leaves = vec![];
            let mut user_end_epochs = vec![];
            for (id, confirmed_spend) in orders {
                leaves.push(confirmed_spend.hash());
                let current_epoch = confirmed_spend.epoch_end.next();
                assert!(u64::from(current_epoch) > 0);

                user_end_epochs.push((confirmed_spend.user, confirmed_spend.epoch_end));
                if let Some(any_mod) = read_inner::<HarvestOrderWrap<StateId>, _>(id, &tx, cf) {
                    match any_mod {
                        AnyMod::Confirmed(Traced {
                            prev_state_id,
                            state: Confirmed(Bundled(mut harvest_order, bearer)),
                        }) => {
                            assert!(matches!(harvest_order.status, HarvestOrderStatus::Unspent));
                            harvest_order.status = HarvestOrderStatus::Spent(current_epoch);

                            let state: Confirmed<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                                Confirmed(Bundled(harvest_order, bearer));
                            write_confirmed_inner(Traced { state, prev_state_id }, &tx, cf);
                        }
                        AnyMod::Predicted(Traced {
                            prev_state_id,
                            state: Predicted(Bundled(mut harvest_order, bearer)),
                        }) => {
                            assert_eq!(harvest_order.status, HarvestOrderStatus::Spent(current_epoch));
                            harvest_order.status = HarvestOrderStatus::Spent(current_epoch);

                            let state: Confirmed<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                                Confirmed(Bundled(harvest_order, bearer));
                            write_confirmed_inner(Traced { state, prev_state_id }, &tx, cf);
                        }
                    }
                }
            }

            let merkle_snapshot_cf = db.cf_handle(CF_MERKLE_TREE_SNAPSHOTS).unwrap();
            let harvest_orders_ending_epoch_cf = db.cf_handle(CF_HARVEST_ORDERS_ENDING_EPOCH).unwrap();

            for (user_key_hash, epoch) in user_end_epochs {
                add_confirmed_harvest_epoch_end(user_key_hash, epoch, &tx, harvest_orders_ending_epoch_cf);
            }

            let mut tree = buffer_read_back::<u8, IndexedMerkleTree>(&db, merkle_snapshot_cf)
                .unwrap()
                .tree;
            tree.append(&mut leaves);
            tree.commit();
            let indexed_tree = IndexedMerkleTree {
                tree,
                slot: Slot(confirmed_slot),
            };
            buffer_push_back::<u8, IndexedMerkleTree>(
                &indexed_tree,
                max_number_merkle_tree_snapshots,
                &tx,
                merkle_snapshot_cf,
            );
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn write_confirmed_refund_harvest_order(&self, id: StateId, slot: Slot) {
        if let Some(any_mod) = self.read::<HarvestOrderWrap<StateId>>(id).await {
            match any_mod {
                AnyMod::Confirmed(Traced {
                    prev_state_id,
                    state: Confirmed(Bundled(mut harvest_order, bearer)),
                }) => {
                    harvest_order.status = HarvestOrderStatus::Refunded(slot);
                    assert!(prev_state_id.is_none());

                    let state: Confirmed<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                        Confirmed(Bundled(harvest_order, bearer));
                    self.write_confirmed(Traced { state, prev_state_id }).await;
                }
                AnyMod::Predicted(Traced {
                    prev_state_id,
                    state: Predicted(Bundled(mut harvest_order, bearer)),
                }) => {
                    harvest_order.status = HarvestOrderStatus::Refunded(slot);
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

    async fn unconsume_confirmed_spent_harvest_orders(
        &self,
        order_ids: Vec<(Ed25519KeyHash, StateId)>,
        confirmed_slot: u64,
    ) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let cf = db.cf_handle(HarvestOrderWrap::<StateId>::ID).unwrap();

            for (_, id) in &order_ids {
                unconsume_harvest_order_inner::<StateId, Bearer>(*id, &tx, cf);
            }
            let merkle_snapshot_cf = db.cf_handle(CF_MERKLE_TREE_SNAPSHOTS).unwrap();

            // Remove last merkle tree.
            let indexed_tree = buffer_pop_back::<u8, IndexedMerkleTree>(&tx, merkle_snapshot_cf).unwrap();
            assert_eq!(indexed_tree.slot.0, confirmed_slot);

            let harvest_orders_ending_epoch_cf = db.cf_handle(CF_HARVEST_ORDERS_ENDING_EPOCH).unwrap();
            for (user_key_hash, _) in order_ids {
                remove_last_confirmed_harvest_epoch_end(user_key_hash, &tx, harvest_orders_ending_epoch_cf);
            }
            tx.commit().unwrap()
        })
        .await
        .unwrap()
    }

    async fn unconsume_harvest_order(&self, id: StateId) {
        let db = self.db.clone();

        spawn_blocking(move || {
            let tx = db.transaction();
            let cf = db.cf_handle(HarvestOrderWrap::<StateId>::ID).unwrap();
            unconsume_harvest_order_inner::<StateId, Bearer>(id, &tx, cf);
            tx.commit().unwrap();
        })
        .await
        .unwrap()
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

    async fn last_epoch_harvested(&self, user: Ed25519KeyHash) -> Option<Epoch> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let cf = db.cf_handle(CF_HARVEST_ORDERS_ENDING_EPOCH).unwrap();
            let mut key = LAST_CONFIRMED_HARVESTED_EPOCH_PREFIX.as_bytes().to_vec();
            key.extend_from_slice(user.to_raw_bytes());
            db.get_cf(cf, key)
                .unwrap()
                .map(|bytes| {
                    let epoch: Vec<Epoch> = rmp_serde::from_slice(&bytes).unwrap();
                    epoch
                })
                .and_then(|epochs| epochs.last().cloned())
        })
        .await
        .unwrap()
    }
    async fn last_confirmed_merkle_tree(&self) -> Option<IndexedMerkleTree> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let merkle_snapshot_cf = db.cf_handle(CF_MERKLE_TREE_SNAPSHOTS).unwrap();
            buffer_read_back::<u8, IndexedMerkleTree>(&db, merkle_snapshot_cf)
        })
        .await
        .unwrap()
    }
}

fn read_inner<T, Bearer>(
    id: T::StableId,
    tx: &Transaction<TransactionDB>,
    cf: &ColumnFamily,
) -> Option<AnyMod<Bundled<T, Bearer>>>
where
    T: unique_ids::UniqueId + EntitySnapshot + Send + Serialize + DeserializeOwned + 'static,
    T::StableId: Serialize + DeserializeOwned + 'static,
    Bearer: Send + 'static + Serialize + DeserializeOwned,
{
    let version_key = prefixed_key(LATEST_VERSION_PREFIX, &id);
    tx.get_cf(cf, version_key).unwrap().and_then(|version_bytes| {
        let mut bytes = rmp_serde::to_vec(&id).unwrap();
        bytes.extend_from_slice(&version_bytes);
        let state_key = prefixed_bytes(STATE_PREFIX, &bytes);
        tx.get_cf(cf, state_key)
            .unwrap()
            .and_then(|state_bytes| rmp_serde::from_slice(&state_bytes).ok())
    })
}

fn write_confirmed_inner<T, Bearer>(
    entity: Traced<Confirmed<Bundled<T, Bearer>>>,
    tx: &Transaction<TransactionDB>,
    cf: &ColumnFamily,
) where
    T: unique_ids::UniqueId + EntitySnapshot + Send + Clone + Serialize + 'static,
    T::StableId: Serialize + DeserializeOwned + 'static,
    Bearer: Send + 'static + Serialize + DeserializeOwned,
{
    let t = entity.state.0 .0.clone();
    let id = t.stable_id();

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
}

fn unconsume_harvest_order_inner<StateId, Bearer>(
    id: StateId,
    tx: &Transaction<TransactionDB>,
    cf: &ColumnFamily,
) where
    StateId: Copy + Eq + Hash + Send + Sync + Debug + Display + Serialize + DeserializeOwned + 'static,
    Bearer: Send + 'static + Serialize + DeserializeOwned,
{
    if let Some(any_mod) = read_inner::<HarvestOrderWrap<StateId>, _>(id, tx, cf) {
        match any_mod {
            AnyMod::Confirmed(Traced {
                prev_state_id,
                state: Confirmed(Bundled(mut harvest_order, bearer)),
            }) => {
                assert!(matches!(
                    harvest_order.status,
                    HarvestOrderStatus::Spent(_) | HarvestOrderStatus::Refunded(_)
                ));
                harvest_order.status = HarvestOrderStatus::Unspent;
                assert!(prev_state_id.is_none());

                let state: Confirmed<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                    Confirmed(Bundled(harvest_order, bearer));
                write_confirmed_inner(Traced { state, prev_state_id }, tx, cf);
            }
            AnyMod::Predicted(Traced {
                prev_state_id,
                state: Predicted(Bundled(mut harvest_order, bearer)),
            }) => {
                assert!(matches!(harvest_order.status, HarvestOrderStatus::Spent(_)));
                harvest_order.status = HarvestOrderStatus::Unspent;
                assert!(prev_state_id.is_none());

                let state: Confirmed<Bundled<HarvestOrderWrap<StateId>, Bearer>> =
                    Confirmed(Bundled(harvest_order, bearer));
                write_confirmed_inner(Traced { state, prev_state_id }, tx, cf);
            }
        }
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

impl<StateId> unique_ids::UniqueId for BufferWalletWrap<StateId> {
    const ID: &str = formatcp!("CF_{}", EntityId::BufferWallet as u8);
}

impl<GaugeId, StateId> unique_ids::UniqueId for Gauge<GaugeId, StateId> {
    const ID: &str = formatcp!("CF_{}", EntityId::Gauge as u8);
}

impl<GaugeId, StateId> unique_ids::UniqueId for AuthManager<GaugeId, StateId> {
    const ID: &str = formatcp!("CF_{}", EntityId::AuthManager as u8);
}

/// Column family ID for a store that maps each user's wallet public key key_hash to the last epoch
/// for which they have harvested SPLASH rewards.
const CF_HARVEST_ORDERS_ENDING_EPOCH: &str = "CF_4";

/// Store of the last N merkle trees, indexed by block slot.
const CF_MERKLE_TREE_SNAPSHOTS: &str = "CF_5";

pub(crate) const COLUMN_FAMILIES: [&str; 6] = [
    <HarvestOrderWrap<u8> as unique_ids::UniqueId>::ID,
    <BufferWalletWrap<u8> as unique_ids::UniqueId>::ID,
    <Gauge<u8, u8> as unique_ids::UniqueId>::ID,
    <AuthManager<u8, u8> as unique_ids::UniqueId>::ID,
    CF_HARVEST_ORDERS_ENDING_EPOCH,
    CF_MERKLE_TREE_SNAPSHOTS,
];

#[repr(u8)]
#[derive(Eq, PartialEq)]
enum EntityId {
    HarvestOrder = 0,
    BufferWallet = 1,
    Gauge = 2,
    AuthManager = 3,
}

// -------------------------------------------------------------------------------------------------

/// Maps (LAST_CONFIRMED_HARVESTED_EPOCH_PREFIX | user_key_hash) to an `Epoch`
const LAST_CONFIRMED_HARVESTED_EPOCH_PREFIX: &str = "e:";

fn add_confirmed_harvest_epoch_end(
    user_key_hash: Ed25519KeyHash,
    epoch: Epoch,
    tx: &Transaction<TransactionDB>,
    cf: &ColumnFamily,
) {
    let mut key = LAST_CONFIRMED_HARVESTED_EPOCH_PREFIX.as_bytes().to_vec();
    key.extend_from_slice(user_key_hash.to_raw_bytes());
    let ending_epochs = if let Some(val_bytes) = tx.get_cf(cf, &key).unwrap() {
        let mut ending_epochs: Vec<Epoch> = rmp_serde::from_slice(&val_bytes).unwrap();
        ending_epochs.push(epoch);
        ending_epochs
    } else {
        vec![epoch]
    };
    tx.put_cf(cf, key, rmp_serde::to_vec_named(&ending_epochs).unwrap())
        .unwrap();
}

fn remove_last_confirmed_harvest_epoch_end(
    user_key_hash: Ed25519KeyHash,
    tx: &Transaction<TransactionDB>,
    cf: &ColumnFamily,
) {
    let mut key = LAST_CONFIRMED_HARVESTED_EPOCH_PREFIX.as_bytes().to_vec();
    key.extend_from_slice(user_key_hash.to_raw_bytes());
    let mut ending_epochs = tx
        .get_cf(cf, &key)
        .unwrap()
        .map(|b| {
            let e: Vec<Epoch> = rmp_serde::from_slice(&b).unwrap();
            e
        })
        .unwrap();
    ending_epochs.pop().unwrap();
    tx.put_cf(cf, key, rmp_serde::to_vec_named(&ending_epochs).unwrap())
        .unwrap();
}

// -------------------------------------------------------------------------------------------------
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
    use cml_chain::{address::RewardAddress, certs::StakeCredential};
    use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
    use rand::{Rng, RngCore};
    use rs_merkle::{algorithms::Keccak256, MerkleTree};
    use spectrum_cardano_lib::address::{PlutusAddress, PlutusCredential};
    use spectrum_offchain::domain::{
        event::{AnyMod, Confirmed, Predicted, Traced},
        EntitySnapshot, Stable,
    };
    use splash_dao_offchain::routines::Slot;

    use crate::entity_index::{
        rocksdb::{HarvestOrderIndex, HarvestOrderStatus, IndexerDB, Mod, OnChainIndex},
        HarvestOrderSpend,
    };
    use splash_yf_offchain::{
        entities::{
            buffer_wallet::{BufferWallet, BufferWalletWrap},
            gauge::Gauge,
            harvest_order::HarvestOrder,
        },
        Epoch,
    };

    const CAPACITY: u8 = 20;

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
            let key_hash = Ed25519KeyHash::from([0; 28]);
            let reward_address = RewardAddress::new(0, StakeCredential::new_pub_key(key_hash)).to_address();
            let spend = HarvestOrderSpend {
                amount: 1000,
                epoch_start: Epoch::from(0),
                epoch_end: Epoch::from(1),
                user: key_hash,
                reward_address,
            };
            <IndexerDB as HarvestOrderIndex<u32, u32>>::write_predicted_spend_harvest_order(&db, id, &spend)
                .await;
            let p: Mod<Bundled<(HarvestOrder<u32>, HarvestOrderStatus), u32>> =
                db.read_harvest_order(id).await.unwrap();
            let Mod::Predicted(Bundled((order, status), bearer)) = p else {
                panic!()
            };
            assert_eq!(
                (order, status),
                (h.0.clone(), HarvestOrderStatus::Spent(Epoch::from(2)))
            );
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
            <IndexerDB as HarvestOrderIndex<u32, u32>>::write_confirmed_refund_harvest_order(
                &db,
                i,
                Slot(1000),
            )
            .await;
            let p: Mod<Bundled<(HarvestOrder<u32>, HarvestOrderStatus), _>> =
                db.read_harvest_order(i).await.unwrap();
            let Bundled(order, bearer) = orders[i as usize].clone();
            let expected = Mod::Confirmed(Bundled((order, HarvestOrderStatus::Refunded(Slot(1000))), bearer));
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
    async fn test_confirmed_spend_harvest_orders() {
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

        let mut leaves = vec![];
        let mut spent_orders = vec![];

        let oo: Vec<_> = orders
            .iter()
            .map(|b| {
                let id = b.1;
                let user = b.0.account_key;
                let reward_address = RewardAddress::new(0, StakeCredential::new_pub_key(user)).to_address();
                let spend = HarvestOrderSpend {
                    amount: 1000,
                    epoch_start: Epoch::from(0),
                    epoch_end: Epoch::from(1),
                    user,
                    reward_address,
                };
                leaves.push(spend.hash());
                spent_orders.push((user, id));
                (id, spend)
            })
            .collect();

        let slot = 2_000_000;
        <IndexerDB as HarvestOrderIndex<u32, u32>>::write_confirmed_spend_harvest_orders(
            &db,
            oo.clone(),
            slot,
        )
        .await;

        for (_, spend) in &oo {
            let end_epoch = <IndexerDB as HarvestOrderIndex<u32, u32>>::last_epoch_harvested(&db, spend.user)
                .await
                .unwrap();
            assert_eq!(end_epoch, Epoch::from(1));
        }

        let indexed_tree = <IndexerDB as HarvestOrderIndex<u32, u32>>::last_confirmed_merkle_tree(&db)
            .await
            .unwrap();

        let mut expected_tree: MerkleTree<Keccak256> = MerkleTree::new();
        expected_tree.append(&mut leaves);
        expected_tree.commit();

        assert_eq!(expected_tree.root().unwrap(), indexed_tree.tree.root().unwrap());
        assert_eq!(indexed_tree.slot.0, slot);

        // Spend
        for h in &orders {
            let id = h.1;
            let p: Mod<Bundled<(HarvestOrder<u32>, HarvestOrderStatus), u32>> =
                db.read_harvest_order(id).await.unwrap();
            let Mod::Confirmed(Bundled((order, status), bearer)) = p else {
                panic!()
            };
            assert_eq!(
                (order, status),
                (h.0.clone(), HarvestOrderStatus::Spent(Epoch::from(2)))
            );
            assert_eq!(h.1, bearer);
        }

        // Rollback the spending
        <IndexerDB as HarvestOrderIndex<u32, u32>>::unconsume_confirmed_spent_harvest_orders(
            &db,
            spent_orders,
            slot,
        )
        .await;

        let indexed_tree = <IndexerDB as HarvestOrderIndex<u32, u32>>::last_confirmed_merkle_tree(&db)
            .await
            .unwrap();
        assert!(indexed_tree.tree.root().is_none());

        for (_, spend) in &oo {
            let end_epoch =
                <IndexerDB as HarvestOrderIndex<u32, u32>>::last_epoch_harvested(&db, spend.user).await;
            assert!(end_epoch.is_none());
        }

        // Refund
        for i in 0..n {
            <IndexerDB as HarvestOrderIndex<u32, u32>>::write_confirmed_refund_harvest_order(
                &db,
                i,
                Slot(1000),
            )
            .await;
            let p: Mod<Bundled<(HarvestOrder<u32>, HarvestOrderStatus), _>> =
                db.read_harvest_order(i).await.unwrap();
            let Bundled(order, bearer) = orders[i as usize].clone();
            let expected = Mod::Confirmed(Bundled((order, HarvestOrderStatus::Refunded(Slot(1000))), bearer));
            assert_eq!(expected, p);
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

        //// Undo the refunds
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
        let e: AnyMod<Bundled<BufferWalletWrap<u32>, u32>> = db.read(id).await.unwrap();
        assert!(matches!(e, AnyMod::Predicted(_)));
        let erased = e.erased();
        assert_eq!(erased.1, traced_wallet.state.0 .1);
        assert_eq!(erased.0.wallet, traced_wallet.state.0 .0.wallet);

        let mut expected_entities = vec![];
        for _ in 0..10 {
            let prev_version = wallet.0.wallet.state_id;
            wallet.0.wallet.state_id += 1;
            let confirmed = mk_traced_confirmed(wallet.0.clone(), wallet.version(), Some(prev_version));
            expected_entities.push(confirmed.clone());
            db.write_confirmed(confirmed.clone()).await;
            let e: AnyMod<Bundled<BufferWalletWrap<u32>, u32>> = db.read(id).await.unwrap();
            if let AnyMod::Confirmed(Traced { state, prev_state_id }) = e {
                // This confirmed entity has same version as the previous predicted.
                assert_eq!(prev_state_id, Some(prev_version));
                assert_eq!(confirmed.state.0 .0.wallet, state.0 .0.wallet);
            } else {
                panic!("");
            }
        }

        // Start removing entities
        let mut expected_prev_version = wallet.version() - 1;

        for expected_entity in expected_entities.into_iter().rev().skip(1) {
            assert_eq!(Some(expected_entity.state.0 .0.stable_id()), Some(id));
            dbg!(expected_entity.state.version());
            let prev_ver = <IndexerDB as OnChainIndex<u32>>::remove::<BufferWalletWrap<u32>>(
                &db,
                id,
                expected_entity.state.version() + 1,
            )
            .await
            .unwrap();
            assert_eq!(prev_ver, expected_prev_version);
            let e: AnyMod<Bundled<BufferWalletWrap<u32>, u32>> = db.read(id).await.unwrap();
            if let AnyMod::Confirmed(Traced { state, prev_state_id }) = e {
                assert_eq!(prev_state_id, Some(expected_prev_version - 1));
                assert_eq!(expected_entity.state.0 .0.wallet, state.0 .0.wallet);
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

    fn mk_buffer_wallet() -> BufferWalletWrap<u32> {
        let mut rng = rand::thread_rng();
        let wallet = BufferWallet {
            state_id: rng.next_u32(),
            balance: 1_000_000,
        };
        BufferWalletWrap {
            wallet,
            predicted_merkle_tree: None,
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
        IndexerDB::new(db_path, CAPACITY)
    }

    fn predicted<T>(t: T, bearer: u32) -> Predicted<Bundled<T, u32>> {
        Predicted(Bundled(t, bearer))
    }

    fn confirmed<T>(t: T, bearer: u32) -> Confirmed<Bundled<T, u32>> {
        Confirmed(Bundled(t, bearer))
    }
}
