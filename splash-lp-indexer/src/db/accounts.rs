use crate::account::AccountInPool;
use crate::db::{
    account_key, from_account_key, from_event_key, RocksDB, ACCOUNTS_CF, ACTIVE_FARMS_CF, AGGREGATE_CF,
    EVENTS_CF, MAX_BLOCK_KEY,
};
use crate::event::{AccountEvent, Deposit, Event, FarmEvent, Harvest, PositionEvent, Redeem};
use async_trait::async_trait;
use cml_chain::certs::Credential;
use either::Either;
use rocksdb::{Direction, IteratorMode, ReadOptions};
use spectrum_offchain_cardano::data::PoolId;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use tokio::task::spawn_blocking;

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
            let aggregates_cf = db.cf_handle(AGGREGATE_CF).unwrap();
            if let Some(max_slot) = tx
                .get_cf(aggregates_cf, MAX_BLOCK_KEY)
                .unwrap()
                .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap())
            {
                {
                    let events_cf = db.cf_handle(EVENTS_CF).unwrap();
                    let readopts = ReadOptions::default();
                    let mut iter = tx.iterator_cf_opt(events_cf, readopts, IteratorMode::Start);
                    let mut current_slot = None;
                    let mut events = Vec::new();
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
                        let event = rmp_serde::from_slice::<Event>(&value).unwrap();
                        events.push(event);
                    }
                    let accounts_cf = db.cf_handle(ACCOUNTS_CF).unwrap();
                    let frames = aggregate_events(events);
                    for (pool, mut pool_frame) in frames {
                        let pool_key = rmp_serde::to_vec(&pool).unwrap();
                        let current_slot = current_slot.unwrap();
                        let active_farms_cf = db.cf_handle(ACTIVE_FARMS_CF).unwrap();
                        for farm_event in pool_frame.farm_events {
                            match farm_event {
                                FarmEvent::FarmActivation(_) => {
                                    let value = rmp_serde::to_vec(&current_slot).unwrap();
                                    tx.put_cf(active_farms_cf, pool_key.clone(), value).unwrap();
                                }
                                FarmEvent::FarmDeactivation(_) => {
                                    tx.delete_cf(active_farms_cf, pool_key.clone()).unwrap();
                                }
                            }
                        }
                        let farm_activated_at = tx
                            .get_cf(active_farms_cf, pool_key.clone())
                            .unwrap()
                            .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap());
                        let mut readopts = ReadOptions::default();
                        readopts.set_iterate_range(rocksdb::PrefixRange(pool_key.clone()));
                        let mut iter = db.iterator_cf_opt(
                            accounts_cf,
                            readopts,
                            IteratorMode::From(&pool_key, Direction::Forward),
                        );
                        let mut accounts_for_update: HashMap<Credential, (AccountInPool, AccountFrame)> =
                            HashMap::new();
                        while let Some(Ok((key, value))) = iter.next() {
                            let (_, account_key) = from_account_key(key.to_vec()).unwrap();
                            let account = rmp_serde::from_slice::<AccountInPool>(&value).unwrap();
                            let updated_account = if let Some(farm_activated_at) = farm_activated_at {
                                account.activated(farm_activated_at)
                            } else {
                                account.deactivated()
                            };
                            let events = pool_frame
                                .account_frames
                                .remove(&account_key)
                                .unwrap_or_else(|| AccountFrame::new());
                            accounts_for_update.insert(account_key, (updated_account, events));
                        }
                        for (new_account_key, account_frame) in pool_frame.account_frames {
                            accounts_for_update.insert(
                                new_account_key,
                                (
                                    AccountInPool::new(current_slot, farm_activated_at.is_some()),
                                    account_frame,
                                ),
                            );
                        }
                        let lp_supply = pool_frame.lp_supply.unwrap();
                        for (acc_key, (account_state, account_frame)) in accounts_for_update {
                            let harvested_acc = account_frame
                                .harvest_events
                                .into_iter()
                                .fold(account_state, |st, ev| st.harvested(ev));
                            let pos_adjusted_acc = harvested_acc.position_adjusted(
                                current_slot,
                                lp_supply,
                                account_frame.position_updates,
                            );
                            let serialized_acc = rmp_serde::to_vec_named(&pos_adjusted_acc).unwrap();
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

fn aggregate_events(events: Vec<Event>) -> HashMap<PoolId, PoolFrame> {
    let mut aggregated_events: HashMap<PoolId, PoolFrame> = HashMap::new();
    for event in events {
        let event_pid = event.pool_id();
        match aggregated_events.entry(event_pid) {
            Entry::Vacant(entry) => {
                let mut new_frame = PoolFrame::new();
                new_frame.apply_event(event);
                entry.insert(new_frame);
            }
            Entry::Occupied(mut entry) => {
                entry.get_mut().apply_event(event);
            }
        };
    }
    aggregated_events
}

struct AccountFrame {
    harvest_events: Vec<Harvest>,
    position_updates: Vec<PositionEvent>,
}

impl AccountFrame {
    fn new() -> Self {
        Self {
            harvest_events: vec![],
            position_updates: vec![],
        }
    }
    fn apply_event(&mut self, event: AccountEvent) -> Option<u64> {
        match event {
            AccountEvent::Position(p) => {
                let lp_supply = p.lp_supply();
                self.position_updates.push(p);
                Some(lp_supply)
            }
            AccountEvent::Harvest(h) => {
                self.harvest_events.push(h);
                None
            }
        }
    }
}

struct PoolFrame {
    farm_events: Vec<FarmEvent>,
    account_frames: HashMap<Credential, AccountFrame>,
    lp_supply: Option<u64>,
}

impl PoolFrame {
    fn new() -> Self {
        Self {
            farm_events: vec![],
            account_frames: Default::default(),
            lp_supply: None,
        }
    }
    fn apply_event(&mut self, event: Event) {
        match event {
            Event::Account(account_event) => {
                let maybe_lp_supply = match self.account_frames.entry(account_event.account()) {
                    Entry::Vacant(acc) => {
                        let mut new_frame = AccountFrame::new();
                        let lp_supply = new_frame.apply_event(account_event);
                        acc.insert(new_frame);
                        lp_supply
                    }
                    Entry::Occupied(mut acc) => acc.get_mut().apply_event(account_event),
                };
                if let Some(lp_supply) = maybe_lp_supply {
                    self.lp_supply.replace(lp_supply);
                }
            }
            Event::FarmEvent(farm_event) => {
                self.farm_events.push(farm_event);
            }
        }
    }
}
