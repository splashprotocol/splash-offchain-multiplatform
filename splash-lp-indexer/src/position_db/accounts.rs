use crate::account::AccountPosition;
use crate::position_db::{
    account_key, cred_index_prefix, from_cred_index_key, get_range_iterator, PositionDB, ACCOUNT_POOLS_CF,
    ACCOUNT_POSITIONS_CF, CURRENT_SLOT_KEY, KV_CF,
};
use cml_chain::certs::Credential;
use spectrum_offchain_cardano::data::PoolId;
use tokio::task::spawn_blocking;

#[async_trait::async_trait]
pub trait Accounts {
    async fn query_account(&self, cred: Credential) -> Option<Vec<(PoolId, AccountPosition)>>;
}

#[async_trait::async_trait]
impl Accounts for PositionDB {
    async fn query_account(&self, cred: Credential) -> Option<Vec<(PoolId, AccountPosition)>> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.snapshot();
            let accounts_cf = db.cf_handle(ACCOUNT_POSITIONS_CF).unwrap();
            let cred_index_cf = db.cf_handle(ACCOUNT_POOLS_CF).unwrap();
            let aggregates_cf = db.cf_handle(KV_CF).unwrap();
            let prefix = cred_index_prefix(cred.clone());
            let current_slot = tx
                .get_cf(aggregates_cf, CURRENT_SLOT_KEY)
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
                    .and_then(|acc| rmp_serde::from_slice::<AccountPosition>(&acc).ok());
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
}
