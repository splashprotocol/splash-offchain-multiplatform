use async_trait::async_trait;
use cml_crypto::RawBytesEncoding;
use rocksdb::OptimisticTransactionDB;
use spectrum_offchain_cardano::data::PoolId;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use std::path::Path;
use std::sync::Arc;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait VoteEscrowIndex {
    async fn bind_gauge(&self, gauge_id: FarmId, pool_id: PoolId);
    async fn get_gauge_binding(&self, gauge_id: FarmId) -> Option<PoolId>;
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
    async fn bind_gauge(&self, gauge_id: FarmId, pool_id: PoolId) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let mut bytes = pool_id.0 .0.to_raw_bytes().to_vec();
            bytes.extend(pool_id.0 .1.as_bytes());
            db.put(gauge_id.0.as_bytes(), bytes).unwrap();
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
}
