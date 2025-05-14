use crate::account::AccountInPool;
use crate::position_db::{
    account_key, cred_index_prefix, from_cred_index_key, get_range_iterator, PositionDB, ACCOUNTS_CF,
    AGGREGATE_CF, CREDS_INDEX_CF, MAX_BLOCK_NUM_KEY,
};
use cml_chain::certs::Credential;
use cml_core::serialization::Serialize;
use cml_core::Slot;
use log::info;
use spectrum_offchain_cardano::data::PoolId;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use tokio::task::spawn_blocking;

#[async_trait::async_trait]
pub trait Accounts {
    async fn lock(&self, key: Credential) -> Option<Slot>;

    async fn get_user_shares(&self, key: Credential) -> Vec<(u64, PoolId)>;

    async fn get_account_pools(&self, account: Credential) -> Vec<PoolId>;
}

#[async_trait::async_trait]
impl Accounts for PositionDB {
    async fn lock(&self, key: Credential) -> Option<Slot> {
        let db = self.db.clone();
        spawn_blocking(move || {
            info!("[Accounts] Attempt ot lock for {}", hex::encode(key.to_cbor_bytes()));
            let tx = db.transaction();
            let accounts_cf = db.cf_handle(ACCOUNTS_CF).unwrap();
            let cred_index_cf = db.cf_handle(CREDS_INDEX_CF).unwrap();
            let aggregates_cf = db.cf_handle(AGGREGATE_CF).unwrap();
            let prefix = cred_index_prefix(key.clone());
            info!("[Accounts] Prefix {}", hex::encode(prefix.clone()));
            let current_slot = tx
                .get_cf(aggregates_cf, MAX_BLOCK_NUM_KEY)
                .unwrap()
                .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap())?;
            let mut iter = get_range_iterator(&db, cred_index_cf, prefix);
            let mut pools = vec![];
            while let Some(Ok((index_value, _))) = iter.next() {
                info!("[Accounts] Got account in pool {}", hex::encode(index_value.clone()));
                let (_, pool) = from_cred_index_key(index_value.to_vec()).unwrap();
                pools.push(pool);
            }
            for pool in pools {
                info!("[Accounts] Processing pool {}", pool);
                let key = account_key(pool, key.clone());
                info!("[Accounts] Account key {}", hex::encode(&key));
                let maybe_account = tx
                    .get_cf(accounts_cf, key.clone())
                    .unwrap()
                    .and_then(|acc| rmp_serde::from_slice::<AccountInPool>(&acc).ok());
                info!("[Accounts] Account exists {}", maybe_account.is_some());
                if let Some(account) = maybe_account {
                    info!("[Accounts] Account {:?}", account);
                    let locked_account = account.lock(current_slot);
                    let updated_account_value = rmp_serde::to_vec_named(&locked_account).unwrap();
                    tx.put_cf(accounts_cf, key, updated_account_value).unwrap();
                }
            }
            tx.commit().unwrap();
            Some(current_slot)
        })
        .await
        .unwrap()
    }

    // todo: use lock api?
    async fn get_user_shares(&self, key: Credential) -> Vec<(u64, PoolId)> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let mut users_share_per_pool: Vec<(u64, PoolId)> = vec![];
            let tx = db.transaction();
            let accounts_cf = db.cf_handle(ACCOUNTS_CF).unwrap();
            let cred_index_cf = db.cf_handle(CREDS_INDEX_CF).unwrap();
            let aggregates_cf = db.cf_handle(AGGREGATE_CF).unwrap();
            let prefix = cred_index_prefix(key.clone());
            let current_slot = tx
                .get_cf(aggregates_cf, MAX_BLOCK_NUM_KEY)
                .unwrap()
                .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap())?;
            let mut iter = get_range_iterator(&db, cred_index_cf, prefix);
            let mut pools = vec![];
            while let Some(Ok((index_value, _))) = iter.next() {
                let (_, pool) = from_cred_index_key(index_value.to_vec()).unwrap();
                pools.push(pool);
            }
            for pool in pools {
                let key = account_key(pool, key.clone());
                let maybe_account = tx
                    .get_cf(accounts_cf, key.clone())
                    .unwrap()
                    .and_then(|acc| rmp_serde::from_slice::<AccountInPool>(&acc).ok());
                if let Some(account) = maybe_account {
                    if let Some(activated_at) = account.activated_at {
                        let prev_avg_share_bps = account.avg_share_bps;
                        let past_period_weight = account.updated_at - activated_at;
                        let curr_period_weight = current_slot - account.updated_at;
                        let curr_share_bps = account.clone().share_bps();
                        let new_avg_share_bps_num =
                            prev_avg_share_bps * past_period_weight + curr_share_bps * curr_period_weight;
                        if let Some(new_avg_share_bps) =
                            new_avg_share_bps_num.checked_div(past_period_weight + curr_period_weight)
                        {
                            users_share_per_pool.push((new_avg_share_bps, pool));
                        }
                    }
                }
            }
            tx.commit().unwrap();
            Some(users_share_per_pool)
        })
        .await
        .unwrap_or(None)
        .unwrap()
        .to_vec()
    }

    async fn get_account_pools(&self, account: Credential) -> Vec<PoolId> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let cred_index_cf = db.cf_handle(CREDS_INDEX_CF).unwrap();
            let prefix = cred_index_prefix(account.clone());
            let mut iter = get_range_iterator(&db, cred_index_cf, prefix);
            let mut pools = vec![];
            while let Some(Ok((index_value, _))) = iter.next() {
                let (_, pool) = from_cred_index_key(index_value.to_vec()).unwrap();
                pools.push(pool);
            }
            pools
        })
        .await
        .unwrap()
    }
}
