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
use tracing_subscriber::fmt::format;

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
        info!(
            "Going to put gauge {} binding",
            hex::encode(gauge_id.0.as_bytes())
        );
        spawn_blocking(move || {
            let value = Vec::from(pool_id);
            info!("Going to put key: {}", hex::encode(gauge_id.0.as_bytes()));
            let result = db.put(gauge_id.0.as_bytes(), value);
            info!("Put result is {}", result.is_ok());
            let get_test = db.get(gauge_id.0.as_bytes()).unwrap();
            info!("Get result is {}", get_test.is_some());
        })
        .await
        .unwrap()
    }
    async fn get_gauge_binding(&self, gauge_id: FarmId) -> Option<PoolId> {
        let db = self.db.clone();
        info!(
            "Going to get gauge {} binding",
            hex::encode(gauge_id.0.as_bytes())
        );
        spawn_blocking(move || {
            info!("Going to get key: {}", hex::encode(gauge_id.0.as_bytes()));
            let some_res = db.get(gauge_id.0.as_bytes()).unwrap();
            info!("Gauge from db {}", some_res.is_some());
            info!(
                "Gauge from db encoded {}",
                some_res
                    .clone()
                    .map(|res| hex::encode(&res))
                    .unwrap_or("unknown".to_string())
            );
            info!(
                "Gauge from db encoded {:?}",
                some_res
                    .clone()
                    .map(|res| PoolId::try_from(&*res).unwrap_or(PoolId::random()))
            );
            some_res.and_then(|bytes| PoolId::try_from(&*bytes).ok())
        })
        .await
        .unwrap()
    }

    async fn add_pre_activated_gauge(&self, gauge_id: FarmId, slot: Slot) {
        let db = self.db.clone();
        info!(
            "Going to put pre activated {} gauge at key {}",
            hex::encode(gauge_id.0.as_bytes()), hex::encode(VoteEscrowDB::gauge_pre_activated_key(gauge_id))
        );
        spawn_blocking(move || {
            let value = rmp_serde::to_vec(&slot).unwrap();
            info!("Going to put key: {}", hex::encode(gauge_id.0.as_bytes()));
            let result = db.put(VoteEscrowDB::gauge_pre_activated_key(gauge_id), value);
            info!("Put result is {}", result.is_ok());
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
