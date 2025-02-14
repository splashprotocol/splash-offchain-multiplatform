use crate::account::AccountInPool;
use crate::db::{
    account_key, cred_index_key, from_account_key, from_event_key, get_range_iterator, sus_event_key,
    RocksDB, ACCOUNTS_CF, ACTIVE_FARMS_CF, AGGREGATE_CF, CREDS_INDEX_CF, EVENTS_CF, MAX_BLOCK_KEY,
    SUS_EVENTS_CF,
};
use crate::onchain::event::{
    AccountEvent, FarmEvent, Harvest, OnChainEvent, PositionEvent, SuspendedPositionEvents,
};
use async_trait::async_trait;
use cml_chain::certs::Credential;
use rocksdb::{IteratorMode, ReadOptions};
use spectrum_offchain_cardano::data::PoolId;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use tokio::task::spawn_blocking;

#[async_trait]
pub trait MatureEvents {
    async fn try_process_mature_events(&self, confirmation_delay_blocks: u64) -> bool;
}

#[async_trait]
impl MatureEvents for RocksDB {
    async fn try_process_mature_events(&self, confirmation_delay_blocks: u64) -> bool {
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
                    let mut iter = tx.iterator_cf_opt(events_cf, ReadOptions::default(), IteratorMode::Start);
                    let mut current_slot = None;
                    let mut events = Vec::new();
                    while let Some(Ok((event_key, value))) = iter.next() {
                        let (slot, _) = from_event_key(event_key.clone().to_vec()).unwrap();
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
                        let event = rmp_serde::from_slice::<OnChainEvent>(&value).unwrap();
                        events.push(event);
                        tx.delete_cf(events_cf, event_key).unwrap();
                    }
                    let accounts_cf = db.cf_handle(ACCOUNTS_CF).unwrap();
                    let cred_index_cf = db.cf_handle(CREDS_INDEX_CF).unwrap();
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
                        let mut iter = get_range_iterator(&db, accounts_cf, pool_key);
                        let mut accounts_for_update: HashMap<Credential, (AccountInPool, AccountFrame)> =
                            HashMap::new();
                        let suspended_events_cf = db.cf_handle(SUS_EVENTS_CF).unwrap();
                        while let Some(Ok((key, value))) = iter.next() {
                            let (_, account_cred) = from_account_key(key.to_vec()).unwrap();
                            let account = rmp_serde::from_slice::<AccountInPool>(&value).unwrap();
                            let updated_account = if let Some(farm_activated_at) = farm_activated_at {
                                account.activated(farm_activated_at)
                            } else {
                                account.deactivated()
                            };
                            let mut account_frame = pool_frame
                                .account_frames
                                .remove(&account_cred)
                                .unwrap_or_else(|| AccountFrame::new());
                            let account_prefix = rmp_serde::to_vec(&account_cred.clone()).unwrap();
                            let mut iter = get_range_iterator(&db, suspended_events_cf, account_prefix);
                            while let Some(Ok((key, value))) = iter.next() {
                                let suspended_events = rmp_serde::from_slice(&value).unwrap();
                                account_frame.suspended_position_events.push(suspended_events);
                                account_frame.suspended_position_events_keys.push(key.to_vec());
                            }
                            accounts_for_update.insert(account_cred, (updated_account, account_frame));
                        }
                        // Left events relate to yet non-existent accounts
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
                        for (account_cred, (account_state, mut account_frame)) in accounts_for_update {
                            let next_account_state =
                                if let Some(first_harvest) = account_frame.harvest_events.pop_front() {
                                    account_frame.suspended_position_events_keys.into_iter().for_each(
                                        |key| {
                                            tx.delete_cf(suspended_events_cf, key).unwrap();
                                        },
                                    );
                                    account_frame.harvest_events.into_iter().fold(
                                        account_state
                                            .harvest(account_frame.suspended_position_events, first_harvest),
                                        |st, ev| st.harvest(vec![], ev),
                                    )
                                } else {
                                    if account_state
                                        .locked_at
                                        .is_some_and(|locked_at| current_slot - locked_at > 10)
                                    {
                                        account_state.unlock(account_frame.suspended_position_events)
                                    } else {
                                        account_state
                                    }
                                };
                            let account_key = account_key(pool, account_cred.clone());
                            let next_account_state = match next_account_state.try_adjust_position(
                                current_slot,
                                lp_supply,
                                account_frame.upstream_position_events,
                            ) {
                                Ok(next) => next,
                                Err((intact_account_state, suspended_events)) => {
                                    let suspended_events_key =
                                        sus_event_key(account_cred.clone(), current_slot);
                                    let suspended_events_value =
                                        rmp_serde::to_vec_named(&suspended_events).unwrap();
                                    tx.put_cf(
                                        suspended_events_cf,
                                        suspended_events_key,
                                        suspended_events_value,
                                    )
                                    .unwrap();
                                    intact_account_state
                                }
                            };
                            let updated_account_value = rmp_serde::to_vec_named(&next_account_state).unwrap();
                            tx.put_cf(accounts_cf, account_key, updated_account_value)
                                .unwrap();
                            let cred_index = cred_index_key(account_cred, pool);
                            tx.put_cf(cred_index_cf, cred_index, vec![]).unwrap();
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

fn aggregate_events(events: Vec<OnChainEvent>) -> HashMap<PoolId, PoolFrame> {
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
    harvest_events: VecDeque<Harvest>,
    suspended_position_events: Vec<SuspendedPositionEvents>,
    suspended_position_events_keys: Vec<Vec<u8>>,
    upstream_position_events: Vec<PositionEvent>,
}

impl AccountFrame {
    fn new() -> Self {
        Self {
            harvest_events: VecDeque::new(),
            suspended_position_events_keys: vec![],
            suspended_position_events: vec![],
            upstream_position_events: vec![],
        }
    }
    fn apply_event(&mut self, event: AccountEvent) -> Option<u64> {
        match event {
            AccountEvent::Position(p) => {
                let lp_supply = p.lp_supply();
                self.upstream_position_events.push(p);
                Some(lp_supply)
            }
            AccountEvent::Harvest(h) => {
                self.harvest_events.push_back(h);
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
    fn apply_event(&mut self, event: OnChainEvent) {
        match event {
            OnChainEvent::Account(account_event) => {
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
            OnChainEvent::FarmEvent(farm_event) => {
                self.farm_events.push(farm_event);
            }
        }
    }
}
