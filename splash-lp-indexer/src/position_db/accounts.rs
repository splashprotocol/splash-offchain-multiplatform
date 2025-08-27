use crate::account::AccountPosition;
use crate::onchain::GaugeWeight;
use crate::position_db::{
    account_positions_key, account_to_pools_index_prefix, gauge_key, get_range_iterator_over_snapshot,
    parse_account_to_pools_index, parse_position_key, position_key, ColumnFamilies, PositionDB,
};
use cml_chain::certs::Credential;
use splash_dao_offchain::entities::onchain::inflation_box::emission_rate;
use splash_yf_offchain::Epoch;
use tokio::task::spawn_blocking;

#[derive(Debug)]
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
            let account_pools =
                get_range_iterator_over_snapshot(&snap, cfs.account_pools, account_pools_index.clone(), account_pools_index)
                    .filter_map(|e| match e {
                        Ok((index, _)) => {
                            let (_, pid) = parse_account_to_pools_index(index.to_vec())?;
                            Some(pid)
                        }
                        Err(_) => None,
                    });
            let mut max_epoch = Epoch::from(0);
            let account_positions: u64 = account_pools.map(|pid| {
                let positions_range_key = account_positions_key(pid, &cred);
                let start_from_key = position_key(pid, &cred, from_epoch_inclusive);
                let pool_positions: u64 = // todo: verify
                    get_range_iterator_over_snapshot(&snap, cfs.account_positions, positions_range_key, start_from_key)
                        .filter_map(|e| match e {
                            Ok((key, value)) => {
                                let (_, _, position_epoch) = parse_position_key(key.to_vec())?;
                                let position =
                                    rmp_serde::from_slice::<AccountPosition>(&value).ok()?;
                                let gauge_key = gauge_key(pid, position_epoch);
                                let gauge_weight = snap.get_cf(cfs.gauge_weights, gauge_key).unwrap().and_then(|v| rmp_serde::from_slice::<GaugeWeight>(&v).ok())?;
                                if position_epoch > max_epoch && gauge_weight.non_zero() {
                                    max_epoch = position_epoch;
                                }
                                Some(gauge_reward_in_epoch(position_epoch, gauge_weight, position.avg_share_bps))
                            }
                            Err(_) => None,
                        }).sum();
                pool_positions
            }).sum();
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
