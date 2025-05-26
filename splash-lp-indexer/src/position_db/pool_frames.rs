use crate::position_db::{PositionDB, ACCOUNTS_CF, POOL_LQ_FRAMES_INDEX_CF};
use async_trait::async_trait;
use rocksdb::ReadOptions;
use spectrum_offchain_cardano::data::PoolId;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait PoolFrames {
    async fn get_pool_lq_supply(&self, pool_id: PoolId) -> Option<u64>;
}

#[async_trait]
impl PoolFrames for PositionDB {
    async fn get_pool_lq_supply(&self, pool_id: PoolId) -> Option<u64> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let pool_lq_frames_cf = db.cf_handle(POOL_LQ_FRAMES_INDEX_CF).unwrap();
            let readopts = ReadOptions::default();
            let lq_supply = if let Ok(Some(raw_lq_value)) = db.get_cf_opt(
                pool_lq_frames_cf,
                &rmp_serde::to_vec(&pool_id).unwrap(),
                &readopts,
            ) {
                rmp_serde::from_slice::<u64>(&raw_lq_value).ok()
            } else {
                None
            };
            lq_supply
        })
        .await
        .unwrap()
    }
}
