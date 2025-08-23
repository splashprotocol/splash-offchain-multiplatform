use crate::account::{LockId, PoolAccountState};
use crate::position_db::{
    account_key, cred_index_prefix, from_cred_index_key, get_range_iterator, PositionDB, ACCOUNTS_CF,
    KV_CF, CREDS_INDEX_CF, MAX_SLOT_KEY,
};
use cml_chain::certs::Credential;
use cml_core::Slot;
use serde::{Deserialize, Serialize};
use spectrum_offchain_cardano::data::PoolId;
use tokio::task::spawn_blocking;

#[async_trait::async_trait]
pub trait Accounts {
    async fn query_account(&self, cred: Credential) -> Option<Vec<(PoolId, PoolAccountState)>>;
    async fn lock(&self, key: Credential, lock_id: LockId) -> Result<(), LockRejection>;

    async fn get_account_pools(&self, account: Credential) -> Vec<PoolId>;
}

#[async_trait::async_trait]
impl Accounts for PositionDB {
    async fn query_account(&self, cred: Credential) -> Option<Vec<(PoolId, PoolAccountState)>> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.snapshot();
            let accounts_cf = db.cf_handle(ACCOUNTS_CF).unwrap();
            let cred_index_cf = db.cf_handle(CREDS_INDEX_CF).unwrap();
            let aggregates_cf = db.cf_handle(KV_CF).unwrap();
            let prefix = cred_index_prefix(cred.clone());
            let current_slot = tx
                .get_cf(aggregates_cf, MAX_SLOT_KEY)
                .unwrap()
                .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap())?;
            let mut iter = get_range_iterator(&db, cred_index_cf, prefix);
            let mut pools = vec![];
            while let Some(Ok((index_value, _))) = iter.next() {
                let (_, pool) = from_cred_index_key(index_value.to_vec()).unwrap();
                pools.push(pool);
            }
            let mut accounts = vec![];
            for pool in pools {
                let key = account_key(pool, cred.clone());
                let maybe_account = tx
                    .get_cf(accounts_cf, key.clone())
                    .unwrap()
                    .and_then(|acc| rmp_serde::from_slice::<PoolAccountState>(&acc).ok());
                if let Some(account) = maybe_account {
                    accounts.push((pool, account));
                }
            }
            if accounts.is_empty() {
                return None;
            }
            Some(accounts)
        })
        .await
        .unwrap()
    }
    async fn lock(&self, cred: Credential, lock_id: LockId) -> Result<(), LockRejection> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let accounts_cf = db.cf_handle(ACCOUNTS_CF).unwrap();
            let cred_index_cf = db.cf_handle(CREDS_INDEX_CF).unwrap();
            let kv_cf = db.cf_handle(KV_CF).unwrap();
            let prefix = cred_index_prefix(cred.clone());
            let current_slot = tx
                .get_cf(kv_cf, MAX_SLOT_KEY)
                .unwrap()
                .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap());
            let current_slot = match current_slot {
                Some(slot) => slot,
                None => return Err(LockRejection::NotSynced),
            };
            let mut iter = get_range_iterator(&db, cred_index_cf, prefix);
            let mut pools = vec![];
            while let Some(Ok((index_value, _))) = iter.next() {
                let (_, pool) = from_cred_index_key(index_value.to_vec()).unwrap();
                pools.push(pool);
            }
            for pool in pools {
                let key = account_key(pool, cred.clone());
                let maybe_account = tx
                    .get_cf(accounts_cf, key.clone())
                    .unwrap()
                    .and_then(|acc| rmp_serde::from_slice::<PoolAccountState>(&acc).ok());
                if let Some(account) = maybe_account {
                    match account.lock(current_slot, lock_id) {
                        Ok(locked_account) => {
                            let updated_account_value = rmp_serde::to_vec_named(&locked_account).unwrap();
                            tx.put_cf(accounts_cf, key, updated_account_value).unwrap();
                        }
                        Err(concurrent_lock_id) => {
                            return Err(LockRejection::ConcurrentLock(concurrent_lock_id))
                        }
                    }
                }
            }
            tx.commit().unwrap();
            Ok(())
        })
        .await
        .unwrap()
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

pub enum LockRejection {
    ConcurrentLock(LockId),
    NotSynced,
}
