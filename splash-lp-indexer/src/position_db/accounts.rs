use std::collections::HashMap;

use crate::account::AccountPosition;
use crate::onchain::event::SuspendedPools;
use crate::onchain::GaugeWeight;
use crate::position_db::{
    account_positions_key, account_to_pools_index_prefix, gauge_key, get_active_pools, get_current_slot,
    get_current_suspended_pools, get_range_iterator_over_snapshot, parse_account_to_pools_index,
    parse_position_key, position_key, ColumnFamilies, PositionDB, CURRENT_SLOT_KEY,
};
use cml_chain::certs::Credential;
use log::trace;
use serde::Serialize;
use spectrum_offchain_cardano::data::PoolId;
use splash_dao_offchain::entities::onchain::inflation_box::emission_rate;
use splash_dao_offchain::routines::Slot;
use splash_yf_offchain::Epoch;
use tokio::task::spawn_blocking;

#[derive(Debug, Serialize)]
pub struct AccountReward {
    pub amount: u64,
    pub latest_epoch_inclusive: Epoch,
}

#[async_trait::async_trait]
pub trait Accounts {
    async fn query_account(&self, cred: Credential, from_epoch_inclusive: Epoch) -> Option<AccountReward>;
}

#[async_trait::async_trait]
impl Accounts for PositionDB {
    async fn query_account(&self, cred: Credential, from_epoch_inclusive: Epoch) -> Option<AccountReward> {
        let db = self.db.clone();
        let confirmation_delay_slots = self.confirmation_delay_slots;
        let epoch_start = self.epoch_start;
        let num_slots_in_epoch = self.num_slots_in_epoch;
        spawn_blocking(move || {
            let cfs = ColumnFamilies::new(&db);
            let snap = db.snapshot();
            let account_pools_index = account_to_pools_index_prefix(&cred);
            let account_pools = get_range_iterator_over_snapshot(
                &snap,
                cfs.account_pools,
                account_pools_index.clone(),
                account_pools_index,
            )
            .filter_map(|e| match e {
                Ok((index, _)) => {
                    let (_, pid) = parse_account_to_pools_index(index.to_vec())?;
                    Some(pid)
                }
                Err(_) => None,
            });
            let current_slot = snap
                .get_cf(cfs.kv, CURRENT_SLOT_KEY)
                .unwrap()
                .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap())
                .unwrap();
            let current_epoch = Epoch::unsafe_from_slot(
                current_slot - confirmation_delay_slots,
                num_slots_in_epoch,
                epoch_start,
            );
            let SuspendedPools(suspended_pools) = {
                let tx = db.transaction();
                get_current_suspended_pools(&tx, cfs.suspended_pools)?
            };
            let mut max_epoch = Epoch::from(0);
            let mut active_pools_by_epoch: HashMap<Epoch, Vec<PoolId>> = HashMap::new();
            let account_positions: u64 = account_pools
                .map(|pid| {
                    if suspended_pools.contains(&pid) {
                        return 0;
                    }

                    let positions_range_key = account_positions_key(pid, &cred);
                    let start_from_key = position_key(pid, &cred, from_epoch_inclusive);
                    let pool_positions: u64 = get_range_iterator_over_snapshot(
                        &snap,
                        cfs.account_positions,
                        positions_range_key,
                        start_from_key,
                    )
                    .filter_map(|e| match e {
                        Ok((key, value)) => {
                            let (_, _, position_epoch) = parse_position_key(key.to_vec())?;
                            let is_active = if let Some(active_pools) =
                                active_pools_by_epoch.get(&position_epoch)
                            {
                                active_pools.contains(&pid)
                            } else {
                                let tx = db.transaction();
                                let active_pools = get_active_pools(&tx, cfs.active_pools, position_epoch)?;
                                let is_active = active_pools.contains(&pid);
                                active_pools_by_epoch.insert(position_epoch, active_pools);
                                is_active
                            };

                            if !is_active {
                                return None;
                            }

                            let position = rmp_serde::from_slice::<AccountPosition>(&value).ok()?;
                            let gauge_key = gauge_key(pid, position_epoch);
                            let gauge_weight = snap
                                .get_cf(cfs.gauge_weights, gauge_key)
                                .unwrap()
                                .and_then(|v| rmp_serde::from_slice::<GaugeWeight>(&v).ok())?;
                            trace!(
                                "position_epoch: {}, max_epoch: {}, current_epoch: {}",
                                position_epoch,
                                max_epoch,
                                current_epoch
                            );
                            if position_epoch > max_epoch
                                && position_epoch < current_epoch
                                && gauge_weight.non_zero()
                            {
                                if position_epoch > max_epoch {
                                    max_epoch = position_epoch;
                                }
                                Some(gauge_reward_in_epoch(
                                    position_epoch,
                                    gauge_weight,
                                    position.weighted_average_share_bps(),
                                ))
                            } else {
                                None
                            }
                        }
                        Err(_) => None,
                    })
                    .sum();
                    pool_positions
                })
                .sum();
            Some(AccountReward {
                amount: account_positions,
                latest_epoch_inclusive: max_epoch,
            })
        })
        .await
        .unwrap()
    }
}

fn gauge_reward_in_epoch(epoch: Epoch, gauge_weight: GaugeWeight, position_share_bps: u64) -> u64 {
    let emission = emission_rate(epoch.unwrap() as u32).untag();
    let gauge_reward = gauge_weight.mul(emission);
    let total_reward = gauge_reward * position_share_bps / 10_000;
    trace!("epoch: {}, gauge_weight: {:?}, position_share_bps: {}, emission: {}, gauge_reward: {}, total_reward: {}", epoch, gauge_weight, position_share_bps, emission, gauge_reward, total_reward);
    total_reward
}

#[cfg(test)]
mod tests {
    use super::*;
    use cml_chain::crypto::Ed25519KeyHash;
    use cml_core::serialization::FromBytes;
    use rand::{Rng, RngCore};
    use rocksdb::{Options, SingleThreaded, TransactionDB, TransactionDBOptions};
    use spectrum_cardano_lib::time::posix_to_slot;
    use spectrum_offchain_cardano::data::PoolId;
    use splash_testing::db_path::DBPath;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_position(qty: u64) -> AccountPosition {
        AccountPosition::new(0, (qty, 2 * qty))
    }

    #[test]
    fn gen_emission() {
        let epochs = [9_u32, 12, 13, 47, 48];
        for epoch in epochs {
            let epoch = Epoch::from(epoch as u64);
            let epoch_start = posix_to_slot(1761549148000 / 1000, 0.into());
            let slots_in_epoch = 28_800;
            let first_slot = epoch.first_slot(slots_in_epoch, epoch_start);
            let last_slot = epoch.last_slot(slots_in_epoch, epoch_start);
            println!(
                "epoch: {}, epoch_start: {}, epoch_end: {}, epoch_len: {}",
                epoch,
                first_slot,
                last_slot,
                last_slot - first_slot
            );
        }
    }

    #[test]
    fn test_range_iterator() {
        let n = DBPath::new("_test_read_max_key");
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db_opts = TransactionDBOptions::default();
        let cf = "test_cf";
        let db = Arc::new(TransactionDB::<SingleThreaded>::open_cf(&opts, &db_opts, &n, [cf]).unwrap());
        let cf = db.cf_handle(cf).unwrap();

        let mut bf = [0u8; 28];
        rand::thread_rng().fill_bytes(&mut bf);

        let random_cred_bytes = Ed25519KeyHash::from(bf);
        let cred = Credential::new_pub_key(random_cred_bytes);
        let pid = PoolId::random();
        let mut rng = rand::thread_rng();

        // Insert test positions for epochs 1-5
        for epoch in 1..=5 {
            let key = position_key(pid, &cred, Epoch::from(epoch));
            let position = create_test_position(rng.gen_range(0..10000));
            db.put_cf(cf, key, rmp_serde::to_vec(&position).unwrap()).unwrap();
        }

        let unrelated_pid = PoolId::random();
        let unrelated_key = position_key(unrelated_pid, &cred, Epoch::from(3));
        let unrelated_position = create_test_position(rng.gen_range(0..10000));
        db.put_cf(cf, unrelated_key, rmp_serde::to_vec(&unrelated_position).unwrap())
            .unwrap();

        let snap = db.snapshot();
        let positions_range_key = account_positions_key(pid, &cred);
        let start_from_key = position_key(pid, &cred, Epoch::from(3));

        let positions: Vec<(Epoch, AccountPosition)> =
            get_range_iterator_over_snapshot(&snap, cf, positions_range_key, start_from_key)
                .filter_map(|e| match e {
                    Ok((key, value)) => {
                        let (_, _, epoch) = parse_position_key(key.to_vec())?;
                        let position = rmp_serde::from_slice(&value).ok()?;
                        Some((epoch, position))
                    }
                    Err(_) => None,
                })
                .collect();

        assert_eq!(positions.len(), 3);
        assert!(positions.iter().all(|(epoch, _)| epoch.unwrap() >= 3));
        assert!(positions
            .iter()
            .find(|(_, pos)| pos == &unrelated_position)
            .is_none());
        assert!(positions.windows(2).all(|w| w[0].0 < w[1].0));
    }
}
