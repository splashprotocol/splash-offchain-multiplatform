use crate::account::{AccountPosition, DefaultEpochSlotConversion};
use crate::feed::event::ExportAccountPositionEvent;
use crate::onchain::event::{GaugeWeighted, OnChainEvent, PositionEvent};
use crate::position_db::{
    account_to_pools_index, export_feed, from_event_key, gauge_key, get_current_slot, get_last_exported_slot,
    get_pool_lp_supply, get_range_iterator, parse_position_key, pool_key, position_key,
    set_last_exported_slot, set_pool_lp_supply, ColumnFamilies, PositionDB,
};
use async_trait::async_trait;
use cml_chain::certs::Credential;
use cml_core::Slot;
use log::{info, trace};
use rocksdb::{ColumnFamily, IteratorMode, ReadOptions, TransactionDB};
use spectrum_offchain_cardano::data::PoolId;
use splash_dao_offchain::constants::time::EPOCH_LEN;
use splash_yf_offchain::Epoch;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait MatureEvents {
    async fn try_process_mature_events(&self) -> bool;
}

#[async_trait]
impl MatureEvents for PositionDB {
    async fn try_process_mature_events(&self) -> bool {
        let db = self.db.clone();
        let confirmation_delay_slots = self.confirmation_delay_slots;
        let slots_in_epoch = EPOCH_LEN / 1000;
        let epoch_start = self.epoch_start;
        spawn_blocking(move || {
            let cfs = ColumnFamilies::new(&db);
            let tx = db.transaction();
            if let Some(max_slot) = get_current_slot(&tx, cfs.kv) {
                {
                    trace!("max slot: {}", max_slot);
                    let mut iter_events =
                        tx.iterator_cf_opt(cfs.events, ReadOptions::default(), IteratorMode::Start);
                    let mut next_mature_slot = None;
                    let mut events = vec![];
                    let mut export_events = vec![];
                    while let Some(Ok((event_key, value))) = iter_events.next() {
                        let (event_slot, _) = from_event_key(event_key.clone().to_vec()).unwrap();
                        trace!("event slot: {}", event_slot);
                        if let Some(next_mature_slot) = next_mature_slot {
                            // We're processing events by one block (slot) at a time
                            if next_mature_slot != event_slot {
                                trace!(
                                    "next mature slot: {} != event slot: {}",
                                    next_mature_slot,
                                    event_slot
                                );
                                break;
                            }
                        } else {
                            if max_slot < event_slot {
                                trace!("max slot: {} < event slot: {}", max_slot, event_slot);
                                return false;
                            }
                            if max_slot - event_slot <= confirmation_delay_slots {
                                trace!(
                                    "max slot: {} - event slot: {} <= confirmation delay slots: {}",
                                    max_slot,
                                    event_slot,
                                    confirmation_delay_slots
                                );
                                return false;
                            }
                            next_mature_slot = Some(event_slot);
                        };
                        let event = rmp_serde::from_slice::<OnChainEvent>(&value).unwrap();
                        trace!("deleting event at slot: {}: {:?}", event_slot, event);
                        if !events.contains(&event) {
                            events.push(event);
                        }
                        tx.delete_cf(cfs.events, event_key).unwrap();
                    }
                    if let Some(current_slot) = next_mature_slot {
                        let last_exported_slot = get_last_exported_slot(&tx, cfs.kv).unwrap_or(0);
                        let should_create_export_events = current_slot > last_exported_slot;
                        let events_by_pool = aggregate_events(events, &tx, &cfs);
                        for (pool_id, pool_events) in events_by_pool {
                            let pool_key = pool_key(pool_id);
                            let old_lp_supply = get_pool_lp_supply(&tx, cfs.pool_lq, pool_id);
                            if let Some(pool_lp_supply) = pool_events.lp_supply.or(old_lp_supply) {
                                println!(
                                    "YYY: pool_events.lp_supply: {:?}, old_lp_supply: {:?}",
                                    pool_events.lp_supply, old_lp_supply
                                );
                                let latest_account_positions =
                                    get_latest_account_positions(&db, cfs.account_positions, pool_key);
                                if current_slot >= epoch_start {
                                    let current_epoch =
                                        Epoch::unsafe_from_slot(current_slot, slots_in_epoch, epoch_start);
                                    for GaugeWeighted {
                                        pool_id,
                                        weight,
                                        epoch,
                                    } in pool_events.gauge_events
                                    {
                                        assert_eq!(current_epoch, epoch);
                                        tx.put_cf(
                                            cfs.gauge_weights,
                                            gauge_key(pool_id, epoch),
                                            rmp_serde::to_vec(&weight).unwrap(),
                                        )
                                        .unwrap();
                                    }
                                    // We only store positions related to active gauges;
                                    if current_epoch >= Epoch::FIRST {
                                        let positions_for_update = prepare_positions_for_update(
                                            pool_events.account_frames,
                                            latest_account_positions,
                                            current_slot,
                                            pool_lp_supply,
                                            slots_in_epoch,
                                            epoch_start,
                                        );
                                        info!("positions for update: {:?}", positions_for_update);
                                        for ((cred, epoch), position) in positions_for_update {
                                            let position_key = position_key(pool_id, &cred, epoch);
                                            let position_value = rmp_serde::to_vec_named(&position).unwrap();
                                            tx.put_cf(cfs.account_positions, position_key, position_value)
                                                .unwrap();
                                            let account_pools_index = account_to_pools_index(&cred, pool_id);
                                            tx.put_cf(cfs.account_pools, account_pools_index, vec![])
                                                .unwrap();
                                            if should_create_export_events {
                                                export_events.push(ExportAccountPositionEvent {
                                                    account_cred: cred,
                                                    pool_id,
                                                    epoch,
                                                    update: position,
                                                });
                                            }
                                        }
                                    }
                                }
                            } else {
                                trace!("pool lp {} supply is zero", pool_id);
                                return false;
                            }
                        }
                        if !export_events.is_empty() {
                            export_feed::batch_append(&tx, export_events, cfs.account_feed_export);
                            set_last_exported_slot(&tx, cfs.kv, current_slot);
                        }
                    } else {
                        assert!(events.is_empty());
                        return false;
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

fn prepare_positions_for_update(
    position_events: Vec<PositionEvent>,
    latest_account_positions: HashMap<Credential, (Epoch, AccountPosition)>,
    current_slot: Slot,
    pool_lp_supply: u64,
    slots_in_epoch: u64,
    epoch_start: Slot,
) -> HashMap<(Credential, Epoch), AccountPosition> {
    let current_epoch = Epoch::unsafe_from_slot(current_slot, slots_in_epoch, epoch_start);
    info!("prepare positions for update. current epoch: {:?}", current_epoch);
    let mut positions_for_update: HashMap<(Credential, Epoch), AccountPosition> = HashMap::new();

    let mut account_positions_in_current_epoch = HashMap::new();

    if let Some(Some((last_epoch, last_slot))) =
        latest_account_positions
            .iter()
            .last()
            .map(|(_, (position_epoch, position))| {
                position
                    .get_current_share()
                    .map(|interval| (*position_epoch, interval.end))
            })
    {
        // All active positions in the pool should have the same endpoints in terms of slot and epoch.
        assert!(latest_account_positions
            .iter()
            .all(|(_, (position_epoch, position))| position
                .get_current_share()
                .map(|interval| (*position_epoch, interval.end))
                == Some((last_epoch, last_slot))));

        // Extend existing positions to the current slot, creating adjacent positions for epochs in
        // between if necessary.
        for (account_cred, (position_epoch, current_position)) in latest_account_positions.iter() {
            let adjacent_epochs = position_epoch.adjacent_epochs(current_epoch);
            println!("adjacent_epochs: {:?}", adjacent_epochs);
            let mut current_position = current_position.clone();

            if *position_epoch == current_epoch {
                assert!(adjacent_epochs.is_empty());
                current_position.extend_current_share_to(current_slot);
                account_positions_in_current_epoch.insert(account_cred.clone(), current_position);
            } else {
                current_position
                    .extend_current_share_to(position_epoch.last_slot(slots_in_epoch, epoch_start));
                positions_for_update
                    .insert((account_cred.clone(), *position_epoch), current_position.clone());

                for epoch in adjacent_epochs {
                    let epoch_first_slot = epoch.first_slot(slots_in_epoch, epoch_start);
                    let epoch_last_slot = epoch.last_slot(slots_in_epoch, epoch_start);
                    let last_current_share = current_position.get_current_share().unwrap().share;
                    let mut current_position = AccountPosition::new(epoch_first_slot, last_current_share);
                    if epoch == current_epoch {
                        current_position.extend_current_share_to(current_slot);
                        account_positions_in_current_epoch.insert(account_cred.clone(), current_position);
                    } else {
                        current_position.extend_current_share_to(epoch_last_slot);
                        positions_for_update.insert((account_cred.clone(), epoch), current_position);
                    }
                }
            }
        }
    }

    let mut new_account_positions: HashMap<Credential, AccountPosition> = HashMap::new();

    let final_lp_supply = position_events.last().unwrap().lp_supply();

    let converter = DefaultEpochSlotConversion::new(slots_in_epoch, epoch_start);

    // Apply all position events
    for position_event in position_events {
        let account_cred = position_event.account();
        if account_positions_in_current_epoch.contains_key(&account_cred) {
            account_positions_in_current_epoch
                .get_mut(&account_cred)
                .unwrap()
                .update_from_user_event(current_slot, position_event, &converter);
        } else if new_account_positions.contains_key(&account_cred) {
            new_account_positions
                .get_mut(&account_cred)
                .unwrap()
                .update_from_user_event(current_slot, position_event, &converter);
        } else {
            let mut position = AccountPosition::new(current_slot, (0, pool_lp_supply));
            position.update_from_user_event(current_slot, position_event, &converter);
            new_account_positions.insert(account_cred.clone(), position);
        }
    }

    for (account_cred, position) in account_positions_in_current_epoch {
        info!(
            "Update existing account position: {:?}, {:?}",
            account_cred, position
        );
        positions_for_update.insert((account_cred, current_epoch), position);
    }

    for (account_cred, position) in new_account_positions {
        info!("New account position: {:?}, {:?}", account_cred, position);
        positions_for_update.insert((account_cred, current_epoch), position);
    }

    for ((_, epoch), position) in positions_for_update.iter_mut() {
        if *epoch == current_epoch {
            position.update_from_external_pool_changes(current_slot, final_lp_supply, &converter);
        }
    }

    positions_for_update
}

fn aggregate_events(
    events: Vec<OnChainEvent>,
    tx: &rocksdb::Transaction<rocksdb::TransactionDB>,
    cfs: &ColumnFamilies,
) -> HashMap<PoolId, EventsByPool> {
    let mut aggregated_events: HashMap<PoolId, EventsByPool> = HashMap::new();
    for event in events {
        let event_pid = event.pool_id();
        match aggregated_events.entry(event_pid) {
            Entry::Vacant(entry) => {
                let mut new_frame = EventsByPool::new();
                let lp_supply = new_frame.apply_event(event);
                if let Some(lp_supply) = lp_supply {
                    set_pool_lp_supply(tx, cfs.pool_lq, event_pid, lp_supply);
                }
                entry.insert(new_frame);
            }
            Entry::Occupied(mut entry) => {
                let lp_supply = entry.get_mut().apply_event(event);
                if let Some(lp_supply) = lp_supply {
                    set_pool_lp_supply(tx, cfs.pool_lq, event_pid, lp_supply);
                }
            }
        };
    }
    aggregated_events
}

/// Returns the latest account positions that have positive share for each account.
fn get_latest_account_positions(
    db: &Arc<TransactionDB>,
    cf: &ColumnFamily,
    pool_key: Vec<u8>,
) -> HashMap<Credential, (Epoch, AccountPosition)> {
    let iter_positions = get_range_iterator(db, cf, pool_key).filter_map(|e| match e {
        Ok((key, value)) => {
            let (_, account_cred, position_epoch) = parse_position_key(key.to_vec())?;
            let current_position = rmp_serde::from_slice::<AccountPosition>(&value).ok()?;
            Some((account_cred, position_epoch, current_position))
        }
        Err(_) => None,
    });
    let mut latest_positions: HashMap<Credential, (Epoch, AccountPosition)> = HashMap::new();
    for (account_cred, position_epoch, current_position) in iter_positions {
        if let Some((latest_epoch, _)) = latest_positions.get(&account_cred) {
            if position_epoch > *latest_epoch && !current_position.is_currently_zero_share() {
                latest_positions.insert(account_cred, (position_epoch, current_position));
            }
        } else {
            latest_positions.insert(account_cred, (position_epoch, current_position));
        }
    }
    latest_positions
}

/// All events that happened for a single pool in a single slot.
#[derive(Debug)]
struct EventsByPool {
    gauge_events: Vec<GaugeWeighted>,
    account_frames: Vec<PositionEvent>,
    lp_supply: Option<u64>,
}

impl EventsByPool {
    fn new() -> Self {
        Self {
            gauge_events: vec![],
            account_frames: Default::default(),
            lp_supply: None,
        }
    }

    /// Returns the LP supply of the pool if it changed.
    fn apply_event(&mut self, event: OnChainEvent) -> Option<u64> {
        match event {
            OnChainEvent::Account(account_event) => {
                let lp_supply = Some(account_event.lp_supply());
                self.account_frames.push(account_event);
                self.lp_supply = lp_supply;
                lp_supply
            }
            OnChainEvent::Gauge(gauge_event) => {
                self.gauge_events.push(gauge_event);
                None
            }
            OnChainEvent::Pool(pool_created) => {
                self.lp_supply.replace(pool_created.supply_lq);
                Some(pool_created.supply_lq)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::account::{DefaultEpochSlotConversion, EpochSlotConversion, ShareInterval};
    use crate::onchain::event::{Deposit, GaugeWeighted, OnChainEvent, PositionEvent, Redeem};
    use crate::onchain::GaugeWeight;
    use crate::position_db::event_log::EventLog;
    use crate::position_db::export_feed::ExportEventFeed;
    use crate::position_db::mature_events::{prepare_positions_for_update, MatureEvents};
    use crate::position_db::PositionDB;
    use cml_chain::certs::Credential;
    use cml_core::Slot;
    use cml_crypto::Ed25519KeyHash;
    use spectrum_offchain_cardano::data::PoolId;
    use splash_testing::db_path::DBPath;
    use splash_yf_offchain::Epoch;
    use std::collections::HashMap;

    #[tokio::test]
    async fn process_export_mature_events() {
        let db_path = DBPath::new("_test_read_max_key");
        let db = PositionDB::new(&db_path, 5, 1000, 0);

        let pid = PoolId::random();
        let account = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));

        let r2 = (1_000u64, 1_000_000u64);
        let r3 = (1_000u64, 2_000_000u64);
        let r4 = (2_000u64, 8_000_000u64);

        // Generate a few OnChainEvents
        let event1 = OnChainEvent::Gauge(GaugeWeighted {
            pool_id: pid,
            weight: GaugeWeight(1, 1),
            epoch: Epoch::from(0),
        });
        let event2 = OnChainEvent::Account(PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account.clone(),
            lp_mint: r2.0,
            lp_supply: r2.1,
        }));
        let event3 = OnChainEvent::Account(PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account.clone(),
            lp_mint: r3.0,
            lp_supply: r3.1,
        }));
        let event4 = OnChainEvent::Account(PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account.clone(),
            lp_mint: r4.0,
            lp_supply: r4.1,
        }));

        db.batch_append(1, vec![event1, event2]).await;
        db.batch_append(10, vec![event3]).await;

        let ok = db.try_process_mature_events().await;

        assert!(ok);

        let Some((sn, export_event_1)) = db.next().await else {
            panic!("No event")
        };
        db.delete(sn).await;
        println!("{:?}", export_event_1);
        assert_eq!(export_event_1.account_cred, account);
        assert_eq!(export_event_1.update.get_current_share().unwrap().share, r2);

        db.batch_append(25, vec![event4]).await;
        let ok = db.try_process_mature_events().await;
        assert!(ok);

        let Some((sn, export_event_2)) = db.next().await else {
            panic!("No event")
        };
        db.delete(sn).await;
        println!("{:?}", export_event_2);
    }

    #[test]
    fn test_multiple_accounts() {
        let pid = PoolId::random();
        let acc0 = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));
        let acc1 = Credential::new_pub_key(Ed25519KeyHash::from([1u8; 28]));
        let acc2 = Credential::new_pub_key(Ed25519KeyHash::from([2u8; 28]));

        let epoch_start: Slot = 100_000;
        let slots_in_epoch: u64 = 1000;

        let initial_pool_lp_supply = 1_000_000_u64;

        let mut pool_lp_supply = initial_pool_lp_supply;

        let make_deposit = |account: &Credential, lp_mint: u64, pool_lp_supply: &mut u64| -> PositionEvent {
            *pool_lp_supply += lp_mint;
            PositionEvent::Deposit(Deposit {
                pool_id: pid,
                account: account.clone(),
                lp_mint,
                lp_supply: *pool_lp_supply,
            })
        };

        let make_redeem = |account: &Credential, lp_burned: u64, pool_lp_supply: &mut u64| -> PositionEvent {
            assert!(*pool_lp_supply >= lp_burned);
            *pool_lp_supply -= lp_burned;
            PositionEvent::Redeem(Redeem {
                pool_id: pid,
                account: account.clone(),
                lp_burned,
                lp_supply: *pool_lp_supply,
            })
        };

        let mut current_slot = epoch_start + 100;

        // acc0 first deposits
        let mint0 = 100_000;
        let mut acc0_balance = mint0;
        let acc0_deposit_slot = current_slot;
        let deposit0 = make_deposit(&acc0, mint0, &mut pool_lp_supply);
        let m = prepare_positions_for_update(
            vec![deposit0.clone()],
            HashMap::default(),
            current_slot,
            pool_lp_supply,
            slots_in_epoch,
            epoch_start,
        );

        assert_eq!(m.len(), 1);
        let ((cred, epoch), position) = m.into_iter().next().unwrap();
        assert_eq!(cred, acc0);
        assert_eq!(epoch, Epoch::from(0));
        let acc0_current_share_interval = position.get_current_share().unwrap();
        let expected = ShareInterval::new(current_slot, current_slot, (mint0, pool_lp_supply));
        assert_eq!(acc0_current_share_interval, expected);

        // acc1 now deposits in the first epoch
        let mint1 = 200_000;
        let mut acc1_balance = mint1;
        current_slot += 100;
        let acc1_deposit_slot = current_slot;
        let deposit1 = make_deposit(&acc1, mint1, &mut pool_lp_supply);
        let m = prepare_positions_for_update(
            vec![deposit1],
            HashMap::from([(acc0.clone(), (Epoch::from(0), position))]),
            current_slot,
            pool_lp_supply,
            slots_in_epoch,
            epoch_start,
        );
        assert_eq!(pool_lp_supply, initial_pool_lp_supply + mint0 + mint1);
        assert_eq!(m.len(), 2);
        let acc0_position = m.get(&(acc0.clone(), Epoch::from(0))).unwrap().clone();
        let acc1_position = m.get(&(acc1.clone(), Epoch::from(0))).unwrap().clone();

        let mut expected_acc0_epoch_0 = vec![
            ShareInterval::new(acc0_deposit_slot, current_slot, (mint0, deposit0.lp_supply())),
            ShareInterval::new(current_slot, current_slot, (mint0, pool_lp_supply)),
        ];
        assert_eq!(acc0_position.share_intervals, expected_acc0_epoch_0);

        let mut expected_acc1_epoch_0 = vec![ShareInterval::new(
            acc1_deposit_slot,
            current_slot,
            (mint1, pool_lp_supply),
        )];
        assert_eq!(acc1_position.share_intervals, expected_acc1_epoch_0);

        // Now redeem 10_000 from acc0, and deposit 20_000 into acc1 within the same slot in epoch
        // 1.
        current_slot += 1000;
        acc0_balance -= 10_000;
        acc1_balance += 20_000;
        let redeem0 = make_redeem(&acc0, 10_000, &mut pool_lp_supply);
        let deposit1 = make_deposit(&acc1, 20_000, &mut pool_lp_supply);
        let deposit1_lp_supply = deposit1.lp_supply();

        let m = prepare_positions_for_update(
            vec![redeem0, deposit1],
            HashMap::from([
                (acc0.clone(), (Epoch::from(0), acc0_position)),
                (acc1.clone(), (Epoch::from(0), acc1_position)),
            ]),
            current_slot,
            pool_lp_supply,
            slots_in_epoch,
            epoch_start,
        );
        assert_eq!(m.len(), 4);

        let converter = DefaultEpochSlotConversion::new(slots_in_epoch, epoch_start);

        // Positions in epoch 0 are extended to the end of the epoch
        let acc0_position_epoch_0 = m.get(&(acc0.clone(), Epoch::from(0))).unwrap().clone();
        expected_acc0_epoch_0[1].extend_to(converter.last_slot(Epoch::from(0)));
        assert_eq!(acc0_position_epoch_0.share_intervals, expected_acc0_epoch_0);

        let acc1_position_epoch_0 = m.get(&(acc1.clone(), Epoch::from(0))).unwrap().clone();
        expected_acc1_epoch_0[0].extend_to(converter.last_slot(Epoch::from(0)));
        assert_eq!(acc1_position_epoch_0.share_intervals, expected_acc1_epoch_0);

        let acc0_position_epoch_1 = m.get(&(acc0.clone(), Epoch::from(1))).unwrap().clone();
        let acc1_position_epoch_1 = m.get(&(acc1.clone(), Epoch::from(1))).unwrap().clone();

        let mut expected_acc0_epoch_1 = vec![
            ShareInterval::new(
                converter.first_slot(Epoch::from(1)),
                current_slot,
                expected_acc0_epoch_0.last().unwrap().share,
            ),
            ShareInterval::new(current_slot, current_slot, (acc0_balance, deposit1_lp_supply)),
        ];
        assert_eq!(acc0_position_epoch_1.share_intervals, expected_acc0_epoch_1);

        let mut expected_acc1_epoch_1 = vec![
            ShareInterval::new(
                converter.first_slot(Epoch::from(1)),
                current_slot,
                expected_acc1_epoch_0.last().unwrap().share,
            ),
            ShareInterval::new(current_slot, current_slot, (acc1_balance, deposit1_lp_supply)),
        ];

        assert_eq!(acc1_position_epoch_1.share_intervals, expected_acc1_epoch_1);

        // Finally, in epoch 4 within the same slot:
        //  - acc2 makes a first deposit of 200_000
        //  - acc0 deposits 100_000
        //  - acc1 redeems everything
        current_slot += 3000;
        acc0_balance += 100_000;
        let deposit_acc2 = make_deposit(&acc2, 200_000, &mut pool_lp_supply);
        let deposit_acc0 = make_deposit(&acc0, 100_000, &mut pool_lp_supply);
        let redeem_acc1 = make_redeem(&acc1, acc1_balance, &mut pool_lp_supply);
        let m = prepare_positions_for_update(
            vec![deposit_acc2, deposit_acc0, redeem_acc1],
            HashMap::from([
                (acc0.clone(), (Epoch::from(1), acc0_position_epoch_1)),
                (acc1.clone(), (Epoch::from(1), acc1_position_epoch_1)),
            ]),
            current_slot,
            pool_lp_supply,
            slots_in_epoch,
            epoch_start,
        );

        let make_single_interval_over_epoch = |epoch: Epoch, share: (u64, u64)| -> Vec<ShareInterval> {
            vec![ShareInterval::new(
                converter.first_slot(epoch),
                converter.last_slot(epoch),
                share,
            )]
        };
        // Check acc0 positions --------------------------------------------------------------------
        expected_acc0_epoch_1
            .last_mut()
            .unwrap()
            .extend_to(converter.last_slot(Epoch::from(1)));
        let acc0_position_epoch_1 = m.get(&(acc0.clone(), Epoch::from(1))).unwrap().clone();
        assert_eq!(acc0_position_epoch_1.share_intervals, expected_acc0_epoch_1);

        //  - Epochs 2 and 3 are constant positions.
        for e in 2..4 {
            assert_eq!(
                m.get(&(acc0.clone(), Epoch::from(e)))
                    .unwrap()
                    .clone()
                    .share_intervals,
                make_single_interval_over_epoch(Epoch::from(e), expected_acc0_epoch_1.last().unwrap().share)
            );
        }

        //  - Epoch 4
        let acc0_position_epoch_4 = m.get(&(acc0.clone(), Epoch::from(4))).unwrap().clone();
        let expected_acc0_epoch_4 = vec![
            ShareInterval::new(
                converter.first_slot(Epoch::from(4)),
                current_slot,
                expected_acc0_epoch_1.last().unwrap().share,
            ),
            ShareInterval::new(current_slot, current_slot, (acc0_balance, pool_lp_supply)),
        ];
        assert_eq!(acc0_position_epoch_4.share_intervals, expected_acc0_epoch_4);

        // Check acc1 positions --------------------------------------------------------------------
        expected_acc1_epoch_1
            .last_mut()
            .unwrap()
            .extend_to(converter.last_slot(Epoch::from(1)));
        let acc1_position_epoch_1 = m.get(&(acc1.clone(), Epoch::from(1))).unwrap().clone();
        assert_eq!(acc1_position_epoch_1.share_intervals, expected_acc1_epoch_1);

        //  - Epochs 2 and 3 are constant positions.
        for e in 2..4 {
            assert_eq!(
                m.get(&(acc1.clone(), Epoch::from(e)))
                    .unwrap()
                    .clone()
                    .share_intervals,
                make_single_interval_over_epoch(Epoch::from(e), expected_acc1_epoch_1.last().unwrap().share)
            );
        }

        //  - Epoch 4 is a zero position.
        let acc1_position_epoch_4 = m.get(&(acc1.clone(), Epoch::from(4))).unwrap().clone();
        let expected_acc1_epoch_4 = vec![
            ShareInterval::new(
                converter.first_slot(Epoch::from(4)),
                current_slot,
                expected_acc1_epoch_1.last().unwrap().share,
            ),
            ShareInterval::new(current_slot, current_slot, (0, pool_lp_supply)),
        ];
        assert_eq!(acc1_position_epoch_4.share_intervals, expected_acc1_epoch_4);
        assert!(acc1_position_epoch_4.is_currently_zero_share());

        // Check acc2 positions --------------------------------------------------------------------
        assert!(!m.contains_key(&(acc2.clone(), Epoch::from(0))));
        assert!(!m.contains_key(&(acc2.clone(), Epoch::from(1))));
        assert!(!m.contains_key(&(acc2.clone(), Epoch::from(2))));
        assert!(!m.contains_key(&(acc2.clone(), Epoch::from(3))));
        let acc2_position_epoch_5 = m.get(&(acc2.clone(), Epoch::from(4))).unwrap().clone();
        let expected_acc2_epoch_5 = vec![ShareInterval::new(
            current_slot,
            current_slot,
            (200_000, pool_lp_supply),
        )];
        assert_eq!(acc2_position_epoch_5.share_intervals, expected_acc2_epoch_5);
    }
}
