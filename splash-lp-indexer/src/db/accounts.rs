use crate::db::{
    read_block_num_from_key_unsafe, read_block_num_unsafe, RocksDB, ACCOUNTS_CF, EVENTS_CF, MAX_BLOCK_KEY,
};
use crate::event::LpEvent;
use async_trait::async_trait;
use cml_chain::certs::Credential;
use rocksdb::{IteratorMode, ReadOptions};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use tokio::task::spawn_blocking;
use crate::account::Account;

#[async_trait]
pub trait Accounts {
    async fn try_update_accounts(&self, confirmation_delay_blocks: u64) -> bool;
}

#[async_trait]
impl Accounts for RocksDB {
    async fn try_update_accounts(&self, confirmation_delay_blocks: u64) -> bool {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let events_cf = db.cf_handle(EVENTS_CF).unwrap();
            if let Some(max_block_num) = tx
                .get_cf(events_cf, MAX_BLOCK_KEY)
                .unwrap()
                .map(read_block_num_unsafe)
            {
                let readopts = ReadOptions::default();
                let mut iter = tx.iterator_cf_opt(events_cf, readopts, IteratorMode::Start);
                let mut current_block = None;
                let mut events_by_account: HashMap<Credential, Vec<LpEvent>> = HashMap::new();
                while let Some(Ok((key, value))) = iter.next() {
                    let block_num = read_block_num_from_key_unsafe(key.clone().to_vec());
                    if let Some(current_block_num) = current_block {
                        if current_block_num != block_num {
                            break;
                        }
                    } else {
                        if max_block_num - block_num <= confirmation_delay_blocks {
                            return false;
                        }
                        current_block = Some(block_num);
                    };
                    let event = rmp_serde::from_slice::<LpEvent>(&value).unwrap();
                    match events_by_account.entry(event.account()) {
                        Entry::Vacant(mut entry) => {
                            entry.insert(vec![event]);
                        }
                        Entry::Occupied(mut entry) => {
                            entry.get_mut().push(event);
                        }
                    }
                }
                let accounts_cf = db.cf_handle(ACCOUNTS_CF).unwrap();
                for (cred, events) in events_by_account {
                    let account_key = rmp_serde::to_vec(&cred).unwrap();
                    let acc = tx
                        .get_cf(accounts_cf, &account_key)
                        .unwrap()
                        .and_then(|bs| rmp_serde::from_slice(&*bs).ok())
                        .unwrap_or_else(|| Account::empty());
                    let updated_acc = events.into_iter().fold(acc, |acc, event| acc.apply_event(event));
                    let serialized_acc = rmp_serde::to_vec_named(&updated_acc).unwrap();
                    tx.put_cf(accounts_cf, account_key, serialized_acc).unwrap();
                }
                return true;
            }
            false
        })
        .await
        .unwrap()
    }
}
