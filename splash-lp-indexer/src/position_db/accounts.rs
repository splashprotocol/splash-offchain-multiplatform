use crate::account::AccountPosition;
use crate::onchain::GaugeWeight;
use crate::position_db::{
    account_positions_key, account_to_pools_index_prefix, gauge_key, get_range_iterator_over_snapshot,
    parse_account_to_pools_index, parse_position_key, position_key, ColumnFamilies, PositionDB,
};
use cml_chain::certs::Credential;
use serde::Serialize;
use splash_dao_offchain::entities::onchain::inflation_box::emission_rate;
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
            let mut max_epoch = Epoch::from(0);
            let account_positions: u64 = account_pools
                .map(|pid| {
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
                            let position = rmp_serde::from_slice::<AccountPosition>(&value).ok()?;
                            let gauge_key = gauge_key(pid, position_epoch);
                            let gauge_weight = snap
                                .get_cf(cfs.gauge_weights, gauge_key)
                                .unwrap()
                                .and_then(|v| rmp_serde::from_slice::<GaugeWeight>(&v).ok())?;
                            if position_epoch > max_epoch && gauge_weight.non_zero() {
                                max_epoch = position_epoch;
                            }
                            Some(gauge_reward_in_epoch(
                                position_epoch,
                                gauge_weight,
                                position.avg_share_bps,
                            ))
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
    gauge_reward * position_share_bps / 10_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use cml_chain::crypto::Ed25519KeyHash;
    use cml_core::serialization::FromBytes;
    use rand::{Rng, RngCore};
    use rocksdb::{Options, SingleThreaded, TransactionDB, TransactionDBOptions};
    use spectrum_offchain_cardano::data::PoolId;
    use splash_testing::db_path::DBPath;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_position(share_bps: u64) -> AccountPosition {
        AccountPosition {
            avg_share_bps: share_bps,
            share: (0, 0),
            created_at: 0,
            updated_at: 0,
            finalized: false,
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
