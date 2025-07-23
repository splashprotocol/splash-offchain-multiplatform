use std::hash::Hash;
use std::{fmt::Display, sync::Arc};

use bloom_offchain::execution_engine::bundled::Bundled;
use cml_crypto::Ed25519KeyHash;
use rocksdb::TransactionDB;
use serde::{de::DeserializeOwned, Serialize};
use spectrum_offchain::domain::Stable;
use spectrum_offchain::domain::{
    event::{AnyMod, Confirmed, Predicted, Traced},
    EntitySnapshot,
};

use crate::onchain::{
    auth_manager::AuthManager, buffer_wallet::BufferWallet, harvest_order::HarvestOrder, smart_farm::Gauge,
};

#[async_trait::async_trait]
pub trait OnChainIndex<Bearer> {
    async fn read<T>(&self, id: T::StableId) -> Option<AnyMod<Bundled<T, Bearer>>>
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send;

    async fn write_predicted<T>(&self, entity: Traced<Predicted<Bundled<T, Bearer>>>)
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send;

    async fn write_confirmed<T>(&self, entity: Traced<Confirmed<Bundled<T, Bearer>>>)
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send;

    /// Deletes latest version of the entity and returns the previous version if it exists.
    async fn remove<T>(&self, version: T::Version) -> Option<T::Version>
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send;
}

pub enum Mod<T> {
    Confirmed(T),
    Predicted(T),
}

#[async_trait::async_trait]
pub trait HarvestOrderIndex<StateId, Bearer>
where
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
{
    async fn read_harvest_order(&self, id: StateId) -> Option<Mod<Bundled<HarvestOrder<StateId>, Bearer>>>;
    async fn write_predicted_harvest_order(&self, order: Predicted<Bundled<HarvestOrder<StateId>, Bearer>>);
    async fn write_confirmed_harvest_order(&self, order: Confirmed<Bundled<HarvestOrder<StateId>, Bearer>>);
    async fn remove<T>(&self, id: StateId) -> Option<StateId>;
}

pub struct IndexerDB {
    pub db: Arc<TransactionDB>,
}

#[async_trait::async_trait]
impl<Bearer> OnChainIndex<Bearer> for IndexerDB
where
    Bearer: Send + 'static,
{
    async fn read<T>(&self, id: T::StableId) -> Option<AnyMod<Bundled<T, Bearer>>>
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send,
    {
        todo!()
    }

    async fn write_predicted<T>(&self, entity: Traced<Predicted<Bundled<T, Bearer>>>)
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send,
    {
        let Bundled(entity, bearer) = entity.state.0;
        let id = T::ID;
        todo!()
    }
    async fn write_confirmed<T>(&self, entity: Traced<Confirmed<Bundled<T, Bearer>>>)
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send,
    {
        todo!()
    }
    /// Deletes latest version(StateId) of the entity and returns the previous version if it exists.
    async fn remove<T>(&self, version: T::Version) -> Option<T::Version>
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send,
    {
        todo!()
    }
}

#[async_trait::async_trait]
impl<StateId, Bearer> HarvestOrderIndex<StateId, Bearer> for IndexerDB
where
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
    Bearer: Send + 'static,
{
    async fn read_harvest_order(&self, id: StateId) -> Option<Mod<Bundled<HarvestOrder<StateId>, Bearer>>> {
        todo!()
    }

    async fn write_predicted_harvest_order(&self, order: Predicted<Bundled<HarvestOrder<StateId>, Bearer>>) {
        todo!()
    }

    async fn write_confirmed_harvest_order(&self, order: Confirmed<Bundled<HarvestOrder<StateId>, Bearer>>) {
        todo!()
    }

    async fn remove<T>(&self, id: StateId) -> Option<StateId> {
        todo!()
    }
}

pub struct HarvestOrderWrap<StateId>(HarvestOrder<StateId>);

impl<StateId> Stable for HarvestOrderWrap<StateId> {
    type StableId = Ed25519KeyHash;

    fn stable_id(&self) -> Self::StableId {
        self.0.account
    }

    fn is_quasi_permanent(&self) -> bool {
        false
    }
}

impl<StateId> EntitySnapshot for HarvestOrderWrap<StateId> {
    type Version = Ed25519KeyHash;

    fn version(&self) -> Self::Version {
        self.0.account
    }
}

mod unique_ids {
    // Sealed trait
    pub trait UniqueId {
        const ID: u8;
    }
}

impl<StateId> unique_ids::UniqueId for HarvestOrderWrap<StateId> {
    const ID: u8 = EntityId::HarvestOrder as u8;
}

impl<StateId> unique_ids::UniqueId for BufferWallet<StateId> {
    const ID: u8 = EntityId::BufferWallet as u8;
}

impl<GaugeId, StateId> unique_ids::UniqueId for Gauge<GaugeId, StateId> {
    const ID: u8 = EntityId::Gauge as u8;
}

impl<GaugeId, StateId> unique_ids::UniqueId for AuthManager<GaugeId, StateId> {
    const ID: u8 = EntityId::AuthManager as u8;
}

#[repr(u8)]
#[derive(Eq, PartialEq)]
enum EntityId {
    HarvestOrder = 0,
    BufferWallet = 1,
    Gauge = 2,
    AuthManager = 3,
}
