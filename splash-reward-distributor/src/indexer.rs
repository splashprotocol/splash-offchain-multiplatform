use std::hash::Hash;
use std::{fmt::Display, sync::Arc};

use bloom_offchain::execution_engine::bundled::Bundled;
use cml_crypto::Ed25519KeyHash;
use rocksdb::TransactionDB;
use serde::{de::DeserializeOwned, Serialize};
use spectrum_offchain::domain::{
    event::{AnyMod, Confirmed, Predicted, Traced},
    EntitySnapshot,
};

use crate::onchain::{
    auth_manager::AuthManager, buffer_wallet::BufferWallet, harvest_order::HarvestOrder, smart_farm::Gauge,
};

#[async_trait::async_trait]
pub trait OnChainIndex<GaugeId, StateId, Bearer>
where
    GaugeId: Copy + Eq + Hash + Send + Sync + Display,
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned,
{
    async fn read_harvest_order(
        &self,
        id: Ed25519KeyHash,
    ) -> Option<AnyMod<Bundled<HarvestOrder<StateId>, Bearer>>>;
    async fn read_buffer_wallet(&self) -> Option<AnyMod<Bundled<BufferWallet<StateId>, Bearer>>>;
    async fn read_gauge(&self, id: GaugeId) -> Option<AnyMod<Bundled<Gauge<GaugeId, StateId>, Bearer>>>;
    async fn read_auth_manager(&self) -> Option<AnyMod<Bundled<AuthManager<GaugeId, StateId>, Bearer>>>;

    async fn write_predicted<T>(&self, entity: Traced<Predicted<Bundled<T, Bearer>>>)
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send;
    async fn write_confirmed<T>(&self, entity: Traced<Confirmed<Bundled<T, Bearer>>>)
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send;

    /// Deletes latest version(StateId) of the entity and returns the previous version if it exists.
    async fn remove<T>(&self, stable_id: T::StableId) -> Option<StateId>
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send;
}

pub struct IndexerDB {
    pub db: Arc<TransactionDB>,
}

#[async_trait::async_trait]
impl<GaugeId, StateId, Bearer> OnChainIndex<GaugeId, StateId, Bearer> for IndexerDB
where
    GaugeId: Copy + Eq + Hash + Send + Sync + Display + 'static,
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned,
    Bearer: Send + 'static,
{
    async fn read_harvest_order(
        &self,
        id: Ed25519KeyHash,
    ) -> Option<AnyMod<Bundled<HarvestOrder<StateId>, Bearer>>> {
        todo!()
    }
    async fn read_buffer_wallet(&self) -> Option<AnyMod<Bundled<BufferWallet<StateId>, Bearer>>> {
        todo!()
    }
    async fn read_gauge(&self, id: GaugeId) -> Option<AnyMod<Bundled<Gauge<GaugeId, StateId>, Bearer>>> {
        todo!()
    }
    async fn read_auth_manager(&self) -> Option<AnyMod<Bundled<AuthManager<GaugeId, StateId>, Bearer>>> {
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
    async fn remove<T>(&self, stable_id: T::StableId) -> Option<StateId>
    where
        T: unique_ids::UniqueId + EntitySnapshot + Send,
    {
        todo!()
    }
}

mod unique_ids {
    // Sealed trait
    pub trait UniqueId {
        const ID: u8;
    }
}

impl<StateId> unique_ids::UniqueId for HarvestOrder<StateId> {
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
