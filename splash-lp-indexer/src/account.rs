use crate::event::{Harvest, PositionEvent};
use cml_core::Slot;
use serde::{Deserialize, Serialize};

// todo: harvest processing

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Debug)]
pub struct AccountInPool {
    /// Accumulator of avg share over period from `created_at` to `activated_at`
    avg_share_bps: u64,
    /// Latest share as (personal_share, total_share)
    share: (u64, u64),
    updated_at: Slot,
    activated_at: Option<Slot>,
}

impl AccountInPool {
    pub fn new(current_slot: Slot, activated: bool) -> Self {
        Self {
            avg_share_bps: 0,
            share: (0, 1),
            updated_at: current_slot,
            activated_at: if activated { Some(current_slot) } else { None },
        }
    }

    pub fn activated(mut self, slot: Slot) -> Self {
        self.activated_at = Some(slot);
        self
    }

    pub fn deactivated(mut self) -> Self {
        self.activated_at = None;
        self
    }

    pub fn harvested(mut self, harvest: Harvest) -> Self {
        match self.activated_at {
            None => self,
            Some(ref mut activated_at) => {
                *activated_at = harvest.harvested_till;
                self
            }
        }
    }

    pub fn position_adjusted(
        mut self,
        current_slot: Slot,
        total_lq: u64,
        events: Vec<PositionEvent>,
    ) -> Self {
        if let Some(activated_at) = self.activated_at {
            let prev_avg_share_bps = self.avg_share_bps;
            let past_period_weight = self.updated_at - activated_at;
            let curr_period_weight = current_slot - self.updated_at;
            let curr_share_bps = self.share_bps();
            let new_avg_share_bps_num =
                prev_avg_share_bps * past_period_weight + curr_share_bps * curr_period_weight;
            let new_avg_share_bps =
                new_avg_share_bps_num.checked_div(past_period_weight + curr_period_weight);
            self.avg_share_bps = new_avg_share_bps.unwrap_or(0);
        }
        let prev_abs_share = self.share.0;
        let new_abs_share = events.into_iter().fold(prev_abs_share, |acc, ev| match ev {
            PositionEvent::Deposit(deposit) => acc.saturating_add(deposit.lp_mint),
            PositionEvent::Redeem(redeem) => acc.saturating_sub(redeem.lp_burned),
        });
        self.share = (new_abs_share, total_lq);
        self.updated_at = current_slot;
        self
    }

    fn share_bps(&self) -> u64 {
        self.share.0 * 10_000 / self.share.1
    }
}

#[cfg(test)]
mod tests {
    use crate::account::AccountInPool;
    use crate::event::{Deposit, PositionEvent};
    use cml_chain::certs::Credential;
    use cml_crypto::Ed25519KeyHash;
    use spectrum_offchain_cardano::data::PoolId;

    #[test]
    fn init_account() {
        let s0 = 10;
        let account_key = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));
        let acc = AccountInPool::new(s0, true);
        let personal_position_lq = 50_000;
        let total_lq = 1_000_000;
        let events = vec![PositionEvent::Deposit(Deposit {
            pool_id: PoolId::random(),
            account: account_key,
            lp_mint: personal_position_lq,
            lp_supply: total_lq,
        })];
        let init_acc = acc.position_adjusted(s0, total_lq, events);
        assert_eq!(
            init_acc,
            AccountInPool {
                avg_share_bps: 0,
                share: (personal_position_lq, total_lq),
                updated_at: s0,
                activated_at: Some(s0),
            }
        )
    }

    #[test]
    fn apply_deposit() {
        let s0 = 10;
        let s1 = 20;
        let s2 = 40;
        let pid = PoolId::random();
        let account_key = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));
        let acc = AccountInPool::new(s0, true);
        let personal_delta_lq_0 = 500_000;
        let total_lq_0 = 1_000_000;
        let events_0 = vec![PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account_key.clone(),
            lp_mint: personal_delta_lq_0,
            lp_supply: total_lq_0,
        })];
        let init_acc = acc.position_adjusted(s0, total_lq_0, events_0);
        let personal_delta_lq_1 = 500_000;
        let total_lq_1 = 4_000_000;
        let events_1 = vec![PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account_key.clone(),
            lp_mint: personal_delta_lq_1,
            lp_supply: total_lq_1,
        })];
        let updated_acc_0 = init_acc.position_adjusted(s1, total_lq_1, events_1);
        let personal_delta_lq_2 = 1_000_000;
        let total_lq_2 = 4_000_000;
        let events_2 = vec![PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account_key,
            lp_mint: personal_delta_lq_1,
            lp_supply: total_lq_1,
        })];
        let updated_acc_1 = updated_acc_0.position_adjusted(s2, total_lq_1, events_2);
        dbg!(updated_acc_1);
    }
}
