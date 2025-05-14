use async_trait::async_trait;
use cml_core::Slot;
use log::info;
use rocksdb::OptimisticTransactionDB;
use spectrum_offchain_cardano::data::PoolId;
use splash_dao_offchain::entities::onchain::poll_factory::PollFactory;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use std::path::Path;
use std::sync::Arc;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait VoteEscrowIndex {
    async fn put_gauge(&self, gauge_id: FarmId, pool_id: PoolId);
    async fn get_gauge_binding(&self, gauge_id: FarmId) -> Option<PoolId>;

    async fn add_pre_activated_gauge(&self, gauge_id: FarmId, slot: Slot);

    async fn get_pre_activated_gauge(&self, gauge_id: FarmId) -> Option<Slot>;

    async fn delete_pre_activated_gauge(&self, gauge_id: FarmId);

    async fn update_poll_factory_snapshot(&self, state: PollFactory);
    async fn get_poll_factory_snapshot(&self) -> Option<PollFactory>;
}

#[derive(Clone)]
pub struct VoteEscrowDB {
    pub db: Arc<OptimisticTransactionDB>,
}

impl VoteEscrowDB {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Self {
        Self {
            db: Arc::new(OptimisticTransactionDB::open_default(db_path).unwrap()),
        }
    }

    pub fn gauge_pre_activated_key(gauge_id: FarmId) -> Vec<u8> {
        let mut key = "pre-activated".as_bytes().to_vec();
        key.extend(gauge_id.0.as_bytes());
        key
    }
}

const POLL_FACTORY_SNAPSHOT_KEY: &[u8] = b"poll_factory_snapshot";

#[async_trait]
impl VoteEscrowIndex for VoteEscrowDB {
    async fn put_gauge(&self, gauge_id: FarmId, pool_id: PoolId) {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.put(gauge_id.0.as_bytes(), Vec::from(pool_id)).unwrap();
        })
        .await
        .unwrap()
    }
    async fn get_gauge_binding(&self, gauge_id: FarmId) -> Option<PoolId> {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.get(gauge_id.0.as_bytes())
                .unwrap()
                .and_then(|bytes| PoolId::try_from(&*bytes).ok())
        })
        .await
        .unwrap()
    }

    async fn add_pre_activated_gauge(&self, gauge_id: FarmId, slot: Slot) {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.put(
                VoteEscrowDB::gauge_pre_activated_key(gauge_id),
                rmp_serde::to_vec(&slot).unwrap(),
            )
            .unwrap();
        })
        .await
        .unwrap()
    }

    async fn get_pre_activated_gauge(&self, gauge_id: FarmId) -> Option<Slot> {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.get(VoteEscrowDB::gauge_pre_activated_key(gauge_id))
                .unwrap()
                .and_then(|bytes| rmp_serde::from_slice(bytes.as_slice()).ok())
        })
        .await
        .unwrap()
    }

    async fn delete_pre_activated_gauge(&self, gauge_id: FarmId) {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.delete(VoteEscrowDB::gauge_pre_activated_key(gauge_id))
                .unwrap()
        })
        .await
        .unwrap()
    }

    async fn update_poll_factory_snapshot(&self, state: PollFactory) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let bytes = rmp_serde::to_vec_named(&state).unwrap();
            db.put(POLL_FACTORY_SNAPSHOT_KEY, bytes).unwrap();
        })
        .await
        .unwrap()
    }
    async fn get_poll_factory_snapshot(&self) -> Option<PollFactory> {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.get(POLL_FACTORY_SNAPSHOT_KEY)
                .unwrap()
                .and_then(|bytes| rmp_serde::from_slice(&bytes).ok())
        })
        .await
        .unwrap()
    }
}
