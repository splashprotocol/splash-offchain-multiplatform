use crate::account::{PoolAccountState, SuspendedPositionEvents};
use crate::feed::event::ExportAccountEvent;
use crate::onchain::event::{
    AccountEvent, AccountPoolHarvested, FarmEvent, MultiAccountHarvested, OnChainEvent, PoolEvent,
    PositionEvent,
};
use crate::position_db::accounts::Accounts;
use crate::position_db::pool_frames::PoolFrames;
use crate::position_db::{
    account_key, cred_index_key, export_feed, from_account_key, from_event_key, get_range_iterator, pool_key,
    sus_event_key, PositionDB, ACCOUNTS_CF, ACCOUNT_FEED_CF, ACTIVE_FARMS_CF, KV_CF, CREDS_INDEX_CF,
    EVENTS_CF, MAX_SLOT_KEY, POOL_LQ_FRAMES_INDEX_CF, SUS_EVENTS_CF,
};
use async_trait::async_trait;
use cml_chain::certs::Credential;
use log::info;
use rocksdb::{IteratorMode, ReadOptions};
use serde::{Deserialize, Serialize};
use spectrum_offchain_cardano::data::PoolId;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use tokio::task::spawn_blocking;

#[async_trait]
pub trait MatureEvents {
    async fn try_process_mature_events(&self, confirmation_delay_blocks: u64) -> bool;
}

#[async_trait]
impl MatureEvents for PositionDB {
    async fn try_process_mature_events(&self, confirmation_delay_blocks: u64) -> bool {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let kv_cf = db.cf_handle(KV_CF).unwrap();
            if let Some(max_slot) = tx
                .get_cf(kv_cf, MAX_SLOT_KEY)
                .unwrap()
                .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap())
            {
                {
                    let events_cf = db.cf_handle(EVENTS_CF).unwrap();
                    let mut iter_events =
                        tx.iterator_cf_opt(events_cf, ReadOptions::default(), IteratorMode::Start);
                    let mut current_slot = None;
                    let mut events = vec![];
                    let mut export_events = vec![];
                    while let Some(Ok((event_key, value))) = iter_events.next() {
                        let (block_num, _) = from_event_key(event_key.clone().to_vec()).unwrap();
                        if let Some(current_slot) = current_slot {
                            if current_slot != block_num {
                                break;
                            }
                        } else {
                            if max_slot - block_num <= confirmation_delay_blocks {
                                return false;
                            }
                            current_slot = Some(block_num);
                        };
                        let event = rmp_serde::from_slice::<OnChainEvent>(&value).unwrap();
                        events.push(event);
                        tx.delete_cf(events_cf, event_key).unwrap();
                    }

                    let accounts_cf = db.cf_handle(ACCOUNTS_CF).unwrap();
                    let cred_index_cf = db.cf_handle(CREDS_INDEX_CF).unwrap();
                    let frames = aggregate_events(events);
                    for (pool_id, mut pool_frame) in frames {
                        let lp_supply;

                        let pool_lq_frames_cf = db.cf_handle(POOL_LQ_FRAMES_INDEX_CF).unwrap();
                        let readopts = ReadOptions::default();

                        // if there is no deposit, redeem events in frame we should restore
                        // previous frame lq_supply
                        if let Some(new_lq_supply) = pool_frame.lp_supply {
                            lp_supply = new_lq_supply
                        } else {
                            if let Ok(Some(raw_lq_value)) = tx.get_cf_opt(
                                pool_lq_frames_cf,
                                &rmp_serde::to_vec(&pool_id).unwrap(),
                                &readopts,
                            ) {
                                lp_supply = rmp_serde::from_slice::<u64>(&raw_lq_value).unwrap();
                            } else {
                                info!(
                                    "No LQ supply found for pool {}. Skip processing events for this frame",
                                    pool_id
                                );
                                continue;
                            }
                        }

                        // update pool lq value
                        tx.put_cf(
                            pool_lq_frames_cf,
                            &rmp_serde::to_vec(&pool_id).unwrap(),
                            &rmp_serde::to_vec(&lp_supply).unwrap(),
                        )
                        .unwrap();

                        let pool_key = pool_key(pool_id);
                        let current_slot = current_slot.unwrap();
                        let active_farms_cf = db.cf_handle(ACTIVE_FARMS_CF).unwrap();
                        for farm_event in pool_frame.farm_events {
                            match farm_event {
                                FarmEvent::FarmActivated(_) => {
                                    let value = rmp_serde::to_vec(&current_slot).unwrap();
                                    tx.put_cf(active_farms_cf, pool_key.clone(), value).unwrap();
                                }
                                FarmEvent::FarmDeactivated(_) => {
                                    tx.delete_cf(active_farms_cf, pool_key.clone()).unwrap();
                                }
                            }
                        }
                        let farm_activated_at = tx
                            .get_cf(active_farms_cf, pool_key.clone())
                            .unwrap()
                            .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap());
                        let mut iter_accounts = get_range_iterator(&db, accounts_cf, pool_key);
                        let mut accounts_for_update: HashMap<Credential, (PoolAccountState, AccountFrame)> =
                            HashMap::new();
                        let suspended_events_cf = db.cf_handle(SUS_EVENTS_CF).unwrap();
                        while let Some(Ok((key, value))) = iter_accounts.next() {
                            let (_, account_cred) = from_account_key(key.to_vec()).unwrap();
                            let account = rmp_serde::from_slice::<PoolAccountState>(&value).unwrap();
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
                            let mut iter_suspended_events =
                                get_range_iterator(&db, suspended_events_cf, account_prefix);
                            while let Some(Ok((key, value))) = iter_suspended_events.next() {
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
                                    PoolAccountState::new(current_slot, farm_activated_at.is_some()),
                                    account_frame,
                                ),
                            );
                        }
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
                                    if account_state.should_unlock(current_slot) {
                                        account_state.unlock(account_frame.suspended_position_events)
                                    } else {
                                        account_state
                                    }
                                };
                            let account_key = account_key(pool_id, account_cred.clone());
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
                            let cred_index = cred_index_key(&account_cred, pool_id);
                            tx.put_cf(cred_index_cf, cred_index, vec![]).unwrap();
                            export_events.push(ExportAccountEvent {
                                account_cred,
                                pool_id,
                                update: next_account_state,
                            });
                        }
                    }
                    let account_feed_cf = db.cf_handle(ACCOUNT_FEED_CF).unwrap();
                    export_feed::batch_append(&tx, export_events, account_feed_cf);
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

#[derive(Debug)]
struct AccountFrame {
    harvest_events: VecDeque<AccountPoolHarvested>,
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

#[derive(Debug)]
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
            OnChainEvent::PoolEvent(pool_event) => match pool_event {
                PoolEvent::PoolCreated(pool_creation_event) => {
                    self.lp_supply.replace(pool_creation_event.supply_lq);
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onchain::event::{Deposit, FarmActivated};
    use crate::position_db::event_log::EventLog;
    use crate::position_db::export_feed::ExportEventFeed;
    use cml_crypto::Ed25519KeyHash;
    use splash_dao_offchain::routines::Slot;
    use splash_testing::db_path::DBPath;

    #[tokio::test]
    async fn process_export_mature_events() {
        let db_path = DBPath::new("_test_read_max_key");
        let db = PositionDB::new(&db_path);

        let pid = PoolId::random();
        let account = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));

        let r2 = (1_000u64, 1_000_000u64);
        let r3 = (1_000u64, 2_000_000u64);
        let r4 = (2_000u64, 8_000_000u64);

        // Generate a few OnChainEvents
        let event1 = OnChainEvent::FarmEvent(FarmEvent::FarmActivated(FarmActivated {
            pool_id: pid,
            slot: Slot(100),
        }));
        let event2 = OnChainEvent::Account(AccountEvent::Position(PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account.clone(),
            lp_mint: r2.0,
            lp_supply: r2.1,
        })));
        let event3 = OnChainEvent::Account(AccountEvent::Position(PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account.clone(),
            lp_mint: r3.0,
            lp_supply: r3.1,
        })));
        let event4 = OnChainEvent::Account(AccountEvent::Position(PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account.clone(),
            lp_mint: r4.0,
            lp_supply: r4.1,
        })));

        db.batch_append(1, vec![event1, event2]).await;
        db.batch_append(10, vec![event3]).await;

        let ok = db.try_process_mature_events(5).await;

        assert!(ok);

        let Some((sn, export_event_1)) = db.next().await else {
            panic!("No event")
        };
        db.delete(sn).await;
        println!("{:?}", export_event_1);
        assert_eq!(export_event_1.account_cred, account);
        assert_eq!(export_event_1.update.share, r2);

        db.batch_append(25, vec![event4]).await;
        let ok = db.try_process_mature_events(5).await;
        assert!(ok);

        let Some((sn, export_event_2)) = db.next().await else {
            panic!("No event")
        };
        db.delete(sn).await;
        println!("{:?}", export_event_2);
    }
}
