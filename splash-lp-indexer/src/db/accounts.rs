use crate::account::AccountInPool;
use crate::db::{
    account_key, from_account_key, from_event_key, RocksDB, ACCOUNTS_CF, EVENTS_CF, MAX_BLOCK_KEY,
};
use crate::event::LpEvent;
use async_trait::async_trait;
use cml_chain::certs::Credential;
use rocksdb::{Direction, IteratorMode, ReadOptions};
use spectrum_offchain_cardano::data::PoolId;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait Accounts {
    async fn try_update_accounts(&self, genesis_slot: u64, confirmation_delay_blocks: u64) -> bool;
}

#[async_trait]
impl Accounts for RocksDB {
    async fn try_update_accounts(&self, genesis_slot: u64, confirmation_delay_blocks: u64) -> bool {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let events_cf = db.cf_handle(EVENTS_CF).unwrap();
            if let Some(max_slot) = tx
                .get_cf(events_cf, MAX_BLOCK_KEY)
                .unwrap()
                .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap())
            {
                {
                    let readopts = ReadOptions::default();
                    let mut iter = tx.iterator_cf_opt(events_cf, readopts, IteratorMode::Start);
                    let mut current_slot = None;
                    let mut aggregated_events: HashMap<PoolId, HashMap<Credential, Vec<LpEvent>>> =
                        HashMap::new();
                    let mut lp_supply_by_pools: HashMap<PoolId, u64> = HashMap::new();
                    while let Some(Ok((key, value))) = iter.next() {
                        let (slot, _) = from_event_key(key.clone().to_vec()).unwrap();
                        if let Some(current_slot) = current_slot {
                            if current_slot != slot {
                                break;
                            }
                        } else {
                            if max_slot - slot <= confirmation_delay_blocks {
                                return false;
                            }
                            current_slot = Some(slot);
                        };
                        let event = rmp_serde::from_slice::<LpEvent>(&value).unwrap();
                        let pid = event.pool_id();
                        lp_supply_by_pools.insert(pid, event.lp_supply());
                        match aggregated_events.entry(event.pool_id()) {
                            Entry::Vacant(mut entry) => {
                                entry.insert(HashMap::from_iter([(event.account(), vec![event])]));
                            }
                            Entry::Occupied(mut entry) => {
                                match entry.get_mut().entry(event.account()) {
                                    Entry::Vacant(mut acc) => {
                                        acc.insert(vec![event]);
                                    }
                                    Entry::Occupied(mut acc) => {
                                        acc.get_mut().push(event);
                                    }
                                };
                            }
                        }
                    }
                    let accounts_cf = db.cf_handle(ACCOUNTS_CF).unwrap();
                    for (pool, mut events_by_account) in aggregated_events {
                        let prefix = rmp_serde::to_vec(&pool).unwrap();
                        let mut readopts = ReadOptions::default();
                        readopts.set_iterate_range(rocksdb::PrefixRange(prefix.clone()));
                        let mut iter = db.iterator_cf_opt(
                            accounts_cf,
                            readopts,
                            IteratorMode::From(&prefix, Direction::Forward),
                        );
                        let mut accounts_for_update: HashMap<Credential, (AccountInPool, Vec<LpEvent>)> =
                            HashMap::new();
                        while let Some(Ok((key, value))) = iter.next() {
                            let (_, account_key) = from_account_key(key.to_vec()).unwrap();
                            let account = rmp_serde::from_slice::<AccountInPool>(&value).unwrap();
                            let events = events_by_account
                                .remove(&account_key)
                                .unwrap_or_else(|| Vec::new());
                            accounts_for_update.insert(account_key, (account, events));
                        }
                        let current_slot = current_slot.unwrap();
                        for (new_account_key, events) in events_by_account {
                            accounts_for_update
                                .insert(new_account_key, (AccountInPool::new(current_slot), events));
                        }
                        let lp_supply = lp_supply_by_pools.remove(&pool).unwrap_or(0u64);
                        for (acc_key, (account_state, events)) in accounts_for_update {
                            let updated_acc = account_state.apply_events(current_slot, lp_supply, events);
                            let serialized_acc = rmp_serde::to_vec_named(&updated_acc).unwrap();
                            let account_key = account_key(pool, acc_key);
                            tx.put_cf(accounts_cf, account_key, serialized_acc).unwrap();
                        }
                    }
                }
                tx.commit().unwrap();
                return true;
            }
            false
        })
        .await
        .unwrap()
    }
}
