use crate::account::AccountPosition;
use crate::feed::event::ExportAccountEvent;
use crate::onchain::event::{AccountPoolHarvested, GaugeEvent, GaugeWeighted, OnChainEvent, PositionEvent};
use crate::position_db::{
    account_key, cred_index_key, export_feed, from_account_key, from_event_key, get_current_slot,
    get_range_iterator, pool_key, sus_event_key, PositionDB, ACCOUNT_FEED_EXPORT_CF, GAUGES,
};
use async_trait::async_trait;
use cml_chain::certs::Credential;
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
            // let cfs = self.column_families();
            // let tx = db.transaction();
            // if let Some(max_slot) = get_current_slot(&tx, cfs.kv) {
            //     {
            //         let mut iter_events =
            //             tx.iterator_cf_opt(cfs.events, ReadOptions::default(), IteratorMode::Start);
            //         let mut next_mature_slot = None;
            //         let mut events = vec![];
            //         let mut export_events = vec![];
            //         while let Some(Ok((event_key, value))) = iter_events.next() {
            //             let (event_slot, _) = from_event_key(event_key.clone().to_vec()).unwrap();
            //             if let Some(next_mature_slot) = next_mature_slot {
            //                 // We're processing slots one at a time
            //                 if next_mature_slot != event_slot {
            //                     break;
            //                 }
            //             } else {
            //                 if max_slot - event_slot <= confirmation_delay_blocks {
            //                     return false;
            //                 }
            //                 next_mature_slot = Some(event_slot);
            //             };
            //             let event = rmp_serde::from_slice::<OnChainEvent>(&value).unwrap();
            //             events.push(event);
            //             tx.delete_cf(cfs.events, event_key).unwrap();
            //         }
            //
            //         let frames = aggregate_events(events);
            //         for (pool_id, mut pool_frame) in frames {
            //             let pool_key = pool_key(pool_id);
            //             let current_slot = next_mature_slot.unwrap();
            //             let active_farms_cf = db.cf_handle(GAUGES).unwrap();
            //             for farm_event in pool_frame.gauge_events {
            //                 todo!("write farm weight")
            //             }
            //             let farm_activated_at = tx
            //                 .get_cf(active_farms_cf, pool_key.clone())
            //                 .unwrap()
            //                 .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap());
            //             let mut iter_accounts = get_range_iterator(&db, cfs.account_positions, pool_key);
            //             let mut accounts_for_update: HashMap<Credential, (AccountPosition, AccountFrame)> =
            //                 HashMap::new();
            //             // while let Some(Ok((key, value))) = iter_accounts.next() {
            //             //     let (_, account_cred) = from_account_key(key.to_vec()).unwrap();
            //             //     let account = rmp_serde::from_slice::<AccountPosition>(&value).unwrap();
            //             //     let updated_account = if let Some(farm_activated_at) = farm_activated_at {
            //             //         account.activated(farm_activated_at)
            //             //     } else {
            //             //         account.deactivated()
            //             //     };
            //             //     let mut account_frame = pool_frame
            //             //         .account_frames
            //             //         .remove(&account_cred)
            //             //         .unwrap_or_else(|| AccountFrame::new());
            //             //     let account_prefix = rmp_serde::to_vec(&account_cred.clone()).unwrap();
            //             //     let mut iter_suspended_events =
            //             //         get_range_iterator(&db, suspended_events_cf, account_prefix);
            //             //     while let Some(Ok((key, value))) = iter_suspended_events.next() {
            //             //         let suspended_events = rmp_serde::from_slice(&value).unwrap();
            //             //         account_frame.suspended_position_events.push(suspended_events);
            //             //         account_frame.suspended_position_events_keys.push(key.to_vec());
            //             //     }
            //             //     accounts_for_update.insert(account_cred, (updated_account, account_frame));
            //             // }
            //             // Left events relate to yet non-existent accounts
            //             // for (new_account_key, account_frame) in pool_frame.account_frames {
            //             //     accounts_for_update.insert(
            //             //         new_account_key,
            //             //         (
            //             //             AccountPosition::new(current_slot, farm_activated_at.is_some()),
            //             //             account_frame,
            //             //         ),
            //             //     );
            //             // }
            //             // for (account_cred, (account_state, mut account_frame)) in accounts_for_update {
            //             //     let next_account_state =
            //             //         if let Some(first_harvest) = account_frame.harvest_events.pop_front() {
            //             //             account_frame.suspended_position_events_keys.into_iter().for_each(
            //             //                 |key| {
            //             //                     tx.delete_cf(suspended_events_cf, key).unwrap();
            //             //                 },
            //             //             );
            //             //             account_frame.harvest_events.into_iter().fold(
            //             //                 account_state
            //             //                     .harvest(account_frame.suspended_position_events, first_harvest),
            //             //                 |st, ev| st.harvest(vec![], ev),
            //             //             )
            //             //         } else {
            //             //             if account_state.should_unlock(current_slot) {
            //             //                 account_state.unlock(account_frame.suspended_position_events)
            //             //             } else {
            //             //                 account_state
            //             //             }
            //             //         };
            //             //     let account_key = account_key(pool_id, account_cred.clone());
            //             //     let next_account_state = match next_account_state.try_adjust_position(
            //             //         current_slot,
            //             //         lp_supply,
            //             //         account_frame.position_events,
            //             //     ) {
            //             //         Ok(next) => next,
            //             //         Err((intact_account_state, suspended_events)) => {
            //             //             let suspended_events_key =
            //             //                 sus_event_key(account_cred.clone(), current_slot);
            //             //             let suspended_events_value =
            //             //                 rmp_serde::to_vec_named(&suspended_events).unwrap();
            //             //             tx.put_cf(
            //             //                 suspended_events_cf,
            //             //                 suspended_events_key,
            //             //                 suspended_events_value,
            //             //             )
            //             //             .unwrap();
            //             //             intact_account_state
            //             //         }
            //             //     };
            //             //     let updated_account_value = rmp_serde::to_vec_named(&next_account_state).unwrap();
            //             //     tx.put_cf(accounts_cf, account_key, updated_account_value)
            //             //         .unwrap();
            //             //     let cred_index = cred_index_key(&account_cred, pool_id);
            //             //     tx.put_cf(cred_index_cf, cred_index, vec![]).unwrap();
            //             //     export_events.push(ExportAccountEvent {
            //             //         account_cred,
            //             //         pool_id,
            //             //         update: next_account_state,
            //             //     });
            //             // }
            //         }
            //         let account_feed_cf = db.cf_handle(ACCOUNT_FEED_EXPORT_CF).unwrap();
            //         export_feed::batch_append(&tx, export_events, account_feed_cf);
            //     }
            //     tx.commit().unwrap();
            //   return true;
            //}
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
    position_events: Vec<PositionEvent>,
}

impl AccountFrame {
    fn new() -> Self {
        Self {
            position_events: vec![],
        }
    }
    fn apply_event(&mut self, event: PositionEvent) -> Option<u64> {
        let lp_supply = event.lp_supply();
        self.position_events.push(event);
        Some(lp_supply)
    }
}

#[derive(Debug)]
struct PoolFrame {
    gauge_events: Vec<GaugeEvent>,
    account_frames: HashMap<Credential, AccountFrame>,
    lp_supply: Option<u64>,
}

impl PoolFrame {
    fn new() -> Self {
        Self {
            gauge_events: vec![],
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
            OnChainEvent::Gauge(gauge_event) => {
                self.gauge_events.push(gauge_event);
            }
            OnChainEvent::Pool(pool_created) => {
                self.lp_supply.replace(pool_created.supply_lq);
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onchain::event::{Deposit};
    use crate::position_db::event_log::EventLog;
    use crate::position_db::export_feed::ExportEventFeed;
    use cml_crypto::Ed25519KeyHash;
    use splash_dao_offchain::routines::Slot;
    use splash_testing::db_path::DBPath;
    //
    // #[tokio::test]
    // async fn process_export_mature_events() {
    //     let db_path = DBPath::new("_test_read_max_key");
    //     let db = PositionDB::new(&db_path);
    //
    //     let pid = PoolId::random();
    //     let account = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));
    //
    //     let r2 = (1_000u64, 1_000_000u64);
    //     let r3 = (1_000u64, 2_000_000u64);
    //     let r4 = (2_000u64, 8_000_000u64);
    //
    //     // Generate a few OnChainEvents
    //     let event1 = OnChainEvent::Gauge(GaugeEvent::FarmActivated(FarmActivated {
    //         pool_id: pid,
    //         slot: Slot(100),
    //     }));
    //     let event2 = OnChainEvent::Account(AccountEvent::Position(PositionEvent::Deposit(Deposit {
    //         pool_id: pid,
    //         account: account.clone(),
    //         lp_mint: r2.0,
    //         lp_supply: r2.1,
    //     })));
    //     let event3 = OnChainEvent::Account(AccountEvent::Position(PositionEvent::Deposit(Deposit {
    //         pool_id: pid,
    //         account: account.clone(),
    //         lp_mint: r3.0,
    //         lp_supply: r3.1,
    //     })));
    //     let event4 = OnChainEvent::Account(AccountEvent::Position(PositionEvent::Deposit(Deposit {
    //         pool_id: pid,
    //         account: account.clone(),
    //         lp_mint: r4.0,
    //         lp_supply: r4.1,
    //     })));
    //
    //     db.batch_append(1, vec![event1, event2]).await;
    //     db.batch_append(10, vec![event3]).await;
    //
    //     let ok = db.try_process_mature_events(5).await;
    //
    //     assert!(ok);
    //
    //     let Some((sn, export_event_1)) = db.next().await else {
    //         panic!("No event")
    //     };
    //     db.delete(sn).await;
    //     println!("{:?}", export_event_1);
    //     assert_eq!(export_event_1.account_cred, account);
    //     assert_eq!(export_event_1.update.share, r2);
    //
    //     db.batch_append(25, vec![event4]).await;
    //     let ok = db.try_process_mature_events(5).await;
    //     assert!(ok);
    //
    //     let Some((sn, export_event_2)) = db.next().await else {
    //         panic!("No event")
    //     };
    //     db.delete(sn).await;
    //     println!("{:?}", export_event_2);
    //}
}
