use crate::account::AccountPosition;
use crate::feed::event::ExportAccountPositionEvent;
use crate::onchain::event::{GaugeWeighted, OnChainEvent, PositionEvent};
use crate::position_db::{
    account_to_pools_index, export_feed, from_event_key, gauge_key, get_current_slot, get_range_iterator,
    parse_position_key, pool_key, position_key, ColumnFamilies, PositionDB,
};
use async_trait::async_trait;
use cml_chain::certs::Credential;
use cml_core::Slot;
use log::trace;
use rocksdb::{IteratorMode, ReadOptions};
use spectrum_offchain_cardano::data::PoolId;
use splash_dao_offchain::constants::time::EPOCH_LEN;
use splash_yf_offchain::Epoch;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
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
                    trace!("current slot: {}", max_slot);
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
                                break;
                            }
                        } else {
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
                        events.push(event);
                        tx.delete_cf(cfs.events, event_key).unwrap();
                    }
                    let events_by_pool = aggregate_events(events);
                    if let Some(current_slot) = next_mature_slot {
                        let current_epoch =
                            Epoch::unsafe_from_slot(current_slot, slots_in_epoch, epoch_start);
                        for (pool_id, pool_events) in events_by_pool {
                            let pool_key = pool_key(pool_id);
                            let pool_lp_supply = pool_events.lp_supply.unwrap_or(0);
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
                            let iter_positions = get_range_iterator(&db, cfs.account_positions, pool_key)
                                .filter_map(|e| match e {
                                    Ok((key, value)) => {
                                        let (_, account_cred, position_epoch) =
                                            parse_position_key(key.to_vec())?;
                                        let current_position =
                                            rmp_serde::from_slice::<AccountPosition>(&value).ok()?;
                                        Some((account_cred, position_epoch.into(), current_position))
                                    }
                                    Err(_) => None,
                                });
                            // We only store positions related to active gauges;
                            if current_epoch >= Epoch::FIRST {
                                let positions_for_update = prepare_positions_for_update(
                                    pool_events.account_frames,
                                    iter_positions,
                                    current_slot,
                                    pool_lp_supply,
                                    slots_in_epoch,
                                    epoch_start,
                                );
                                for ((cred, epoch), position) in positions_for_update {
                                    let position_key = position_key(pool_id, &cred, epoch);
                                    let position_value = rmp_serde::to_vec_named(&position).unwrap();
                                    tx.put_cf(cfs.account_positions, position_key, position_value)
                                        .unwrap();
                                    let account_pools_index = account_to_pools_index(&cred, pool_id);
                                    tx.put_cf(cfs.account_pools, account_pools_index, vec![]).unwrap();
                                    export_events.push(ExportAccountPositionEvent {
                                        account_cred: cred,
                                        pool_id,
                                        epoch,
                                        update: position,
                                    });
                                }
                            }
                        }
                        export_feed::batch_append(&tx, export_events, cfs.account_feed_export);
                    } else {
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
    mut account_updates: HashMap<Credential, EventsByAccount>,
    existing_positions: impl Iterator<Item = (Credential, Epoch, AccountPosition)>,
    current_slot: Slot,
    pool_lp_supply: u64,
    slots_in_epoch: u64,
    epoch_start: Slot,
) -> HashMap<(Credential, Epoch), AccountPosition> {
    let current_epoch = Epoch::unsafe_from_slot(current_slot, slots_in_epoch, epoch_start);
    let mut positions_for_update: HashMap<(Credential, Epoch), AccountPosition> = HashMap::new();
    // Update positions for existing accounts
    for (account_cred, position_epoch, current_position) in existing_positions {
        let account_frame = account_updates
            .remove(&account_cred)
            .unwrap_or_else(|| EventsByAccount::new());
        if position_epoch == current_epoch {
            assert!(!current_position.finalized);
            let updated_position =
                current_position.updated(current_slot, pool_lp_supply, account_frame.position_events);
            positions_for_update.insert((account_cred, current_epoch), updated_position);
        } else {
            let position_epoch = Epoch::from(position_epoch);
            let last_slot_of_the_epoch = position_epoch.last_slot(slots_in_epoch, epoch_start);
            let adjacent_epochs = position_epoch.next().adjacent_epochs(slots_in_epoch, epoch_start);
            // | finalized | adjacent[] | current |
            // subsequent[] - new past positions created while the pool was intact
            let (finalized_position, adjacent_positions, current_position) = finalize_position(
                current_position,
                last_slot_of_the_epoch,
                current_slot,
                pool_lp_supply,
                adjacent_epochs
                    .iter()
                    .map(|e| {
                        (
                            e.first_slot(slots_in_epoch, epoch_start),
                            e.last_slot(slots_in_epoch, epoch_start),
                        )
                    })
                    .collect(),
                account_frame.position_events,
            );
            positions_for_update.insert(
                (account_cred.clone(), Epoch::from(position_epoch)),
                finalized_position,
            );
            for (position, epoch) in adjacent_positions.into_iter().zip(adjacent_epochs.iter()) {
                positions_for_update.insert((account_cred.clone(), *epoch), position);
            }
            positions_for_update.insert((account_cred, current_epoch), current_position);
        }
    }
    // Create positions for new accounts
    for (new_account_key, account_frame) in account_updates {
        positions_for_update.insert(
            (new_account_key, current_epoch),
            AccountPosition::new(current_slot).updated(
                current_slot,
                pool_lp_supply,
                account_frame.position_events,
            ),
        );
    }
    positions_for_update
}

/// Finalizes the given position, creates past adjacent positions and initializes a new one.
/// Returns a triple of the finalized position, past adjacent positions created while the pool
/// remained intact and a fresh position.
fn finalize_position(
    pos: AccountPosition,
    finalized_at: Slot,
    current_slot: Slot,
    current_lq_supply: u64,
    adjacent_epochs: Vec<(Slot, Slot)>,
    events: Vec<PositionEvent>,
) -> (AccountPosition, Vec<AccountPosition>, AccountPosition) {
    let share = pos.share;
    let mut finalized_position = pos.updated(finalized_at, share.1, vec![]);
    finalized_position.finalized = true;
    let current_position = AccountPosition {
        avg_share_bps: finalized_position.avg_share_bps,
        share,
        created_at: adjacent_epochs
            .last()
            .map(|(_, close)| *close + 1)
            .unwrap_or(finalized_at + 1),
        updated_at: current_slot,
        finalized: false,
    }
    .updated(current_slot, current_lq_supply, events);
    let adjacent_positions = adjacent_epochs
        .into_iter()
        .map(|(start, end)| {
            AccountPosition {
                avg_share_bps: finalized_position.avg_share_bps,
                share,
                created_at: start,
                updated_at: end,
                finalized: true,
            }
            .updated(end, share.1, vec![])
        })
        .collect();
    (finalized_position, adjacent_positions, current_position)
}

fn aggregate_events(events: Vec<OnChainEvent>) -> HashMap<PoolId, EventsByPool> {
    let mut aggregated_events: HashMap<PoolId, EventsByPool> = HashMap::new();
    for event in events {
        let event_pid = event.pool_id();
        match aggregated_events.entry(event_pid) {
            Entry::Vacant(entry) => {
                let mut new_frame = EventsByPool::new();
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

/// All events that happened for a single account in a single pool in a single slot.
#[derive(Debug)]
struct EventsByAccount {
    position_events: Vec<PositionEvent>,
}

impl EventsByAccount {
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

/// All events that happened for a single pool in a single slot.
#[derive(Debug)]
struct EventsByPool {
    gauge_events: Vec<GaugeWeighted>,
    account_frames: HashMap<Credential, EventsByAccount>,
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
    fn apply_event(&mut self, event: OnChainEvent) {
        match event {
            OnChainEvent::Account(account_event) => {
                let maybe_lp_supply = match self.account_frames.entry(account_event.account()) {
                    Entry::Vacant(acc) => {
                        let mut new_frame = EventsByAccount::new();
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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::account::AccountPosition;
    use crate::onchain::event::{Deposit, GaugeWeighted, OnChainEvent, PositionEvent};
    use crate::onchain::GaugeWeight;
    use crate::position_db::event_log::EventLog;
    use crate::position_db::export_feed::ExportEventFeed;
    use crate::position_db::mature_events::{prepare_positions_for_update, EventsByAccount, MatureEvents};
    use crate::position_db::PositionDB;
    use cml_chain::certs::Credential;
    use cml_crypto::Ed25519KeyHash;
    use spectrum_offchain_cardano::data::PoolId;
    use splash_testing::db_path::DBPath;
    use splash_yf_offchain::Epoch;
    use std::collections::HashMap;

    #[tokio::test]
    async fn process_export_mature_events() {
        let db_path = DBPath::new("_test_read_max_key");
        let db = PositionDB::new(&db_path, 5, 0);

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
        assert_eq!(export_event_1.update.share, r2);

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
    fn create_adjacent_positions() {
        let pid = PoolId::random();
        let account = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));
        let existing_position = AccountPosition {
            avg_share_bps: 1000,
            share: (50, 100),
            created_at: 0,
            updated_at: 10,
            finalized: false,
        };
        let deposit = PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account.clone(),
            lp_mint: 60,
            lp_supply: 200,
        });
        let positions_for_update = prepare_positions_for_update(
            HashMap::from([(
                account.clone(),
                EventsByAccount {
                    position_events: vec![deposit],
                },
            )]),
            vec![(account.clone(), Epoch::from(1), existing_position)].into_iter(),
            37,
            200,
            10,
            0,
        );
        dbg!(&positions_for_update
            .iter()
            .map(|(k, v)| (format!("-, {}", k.1), v))
            .collect::<Vec<_>>());
    }
}
