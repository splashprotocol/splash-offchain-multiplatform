use async_trait::async_trait;
use rocksdb::OptimisticTransactionDB;
use spectrum_offchain_cardano::data::PoolId;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use std::path::Path;
use std::sync::Arc;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait GaugeIndex {
    async fn put_gauge(&self, gauge_id: FarmId, pool_id: PoolId);
    async fn get_gauge_binding(&self, gauge_id: FarmId) -> Option<PoolId>;
}

#[derive(Clone)]
pub struct GaugeIndexDB {
    pub db: Arc<OptimisticTransactionDB>,
}

impl GaugeIndexDB {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Self {
        Self {
            db: Arc::new(OptimisticTransactionDB::open_default(db_path).unwrap()),
        }
    }
}

#[async_trait]
impl GaugeIndex for GaugeIndexDB {
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
}
