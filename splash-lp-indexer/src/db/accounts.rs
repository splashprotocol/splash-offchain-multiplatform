use crate::account::AccountInPool;
use crate::db::{
    account_key, cred_index_prefix, from_cred_index_key, get_range_iterator, RocksDB, ACCOUNTS_CF,
    AGGREGATE_CF, CREDS_INDEX_CF, MAX_BLOCK_KEY,
};
use cml_chain::certs::Credential;
use cml_core::Slot;
use tokio::task::spawn_blocking;

#[async_trait::async_trait]
pub trait Accounts {
    async fn lock(&self, key: Credential) -> Option<Slot>;
}

#[async_trait::async_trait]
impl Accounts for RocksDB {
    async fn lock(&self, key: Credential) -> Option<Slot> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let accounts_cf = db.cf_handle(ACCOUNTS_CF).unwrap();
            let cred_index_cf = db.cf_handle(CREDS_INDEX_CF).unwrap();
            let aggregates_cf = db.cf_handle(AGGREGATE_CF).unwrap();
            let prefix = cred_index_prefix(key.clone());
            let current_slot = tx
                .get_cf(aggregates_cf, MAX_BLOCK_KEY)
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
}
