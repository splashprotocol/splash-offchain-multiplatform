use crate::onchain::event::PositionEvent;
use cml_core::Slot;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Eq, PartialEq, Serialize, Deserialize, Debug)]
pub struct AccountPosition {
    /// Accumulator of avg share over period from `updated_at` to `updated_at`
    pub avg_share_bps: u64,
    /// Latest share as (personal_share, total_share)
    pub share: (u64, u64),
    pub created_at: Slot,
    pub updated_at: Slot,
}

impl AccountPosition {
    pub fn new(current_slot: Slot) -> Self {
        Self {
            avg_share_bps: 0,
            share: (0, 1),
            created_at: current_slot,
            updated_at: current_slot,
        }
    }

    pub fn adjusted(mut self, current_slot: Slot, total_lq: u64, events: Vec<PositionEvent>) -> Self {
        let prev_avg_share_bps = self.avg_share_bps;
        let past_period_weight = self.updated_at - self.created_at;
        let curr_period_weight = current_slot - self.updated_at;
        let curr_share_bps = self.share_bps();
        let new_avg_share_bps_num =
            prev_avg_share_bps * past_period_weight + curr_share_bps * curr_period_weight;
        let new_avg_share_bps = new_avg_share_bps_num.checked_div(past_period_weight + curr_period_weight);
        self.avg_share_bps = new_avg_share_bps.unwrap_or(0);
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
    use crate::account::AccountPosition;
    use crate::onchain::event::{Deposit, PositionEvent, Redeem};
    use cml_chain::certs::Credential;
    use cml_crypto::Ed25519KeyHash;
    use spectrum_offchain_cardano::data::PoolId;

    #[test]
    fn init_account() {
        let s0 = 10;
        let account_key = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));
        let acc = AccountPosition::new(s0);
        let personal_position_lq = 50_000;
        let total_lq = 1_000_000;
        let events = vec![PositionEvent::Deposit(Deposit {
            pool_id: PoolId::random(),
            account: account_key,
            lp_mint: personal_position_lq,
            lp_supply: total_lq,
        })];
        let init_acc = acc.adjusted(s0, total_lq, events);
        assert_eq!(
            init_acc,
            AccountPosition {
                avg_share_bps: 0,
                share: (personal_position_lq, total_lq),
                created_at: s0,
                updated_at: s0,
            }
        );
    }

    #[test]
    fn deposit_redeem() {
        let s0 = 10;
        let s1 = 20;
        let s2 = 30;
        let pid = PoolId::random();
        let account_key = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));
        let acc = AccountPosition::new(s0);
        let personal_delta_lq_0 = 500_000;
        let total_lq_0 = 1_000_000;
        let events_0 = vec![PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account_key.clone(),
            lp_mint: personal_delta_lq_0,
            lp_supply: total_lq_0,
        })];
        let init_acc = acc.adjusted(s0, total_lq_0, events_0);
        let personal_delta_lq_1 = 500_000;
        let total_lq_1 = 4_000_000;
        let events_1 = vec![PositionEvent::Redeem(Redeem {
            pool_id: pid,
            account: account_key.clone(),
            lp_burned: personal_delta_lq_1,
            lp_supply: total_lq_1,
        })];
        let updated_acc_0 = init_acc.adjusted(s1, total_lq_1, events_1);
        let personal_delta_lq_2 = 1_000_000;
        let total_lq_2 = 4_000_000;
        let events_2 = vec![PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account_key,
            lp_mint: personal_delta_lq_2,
            lp_supply: total_lq_2,
        })];
        let updated_acc_2 = updated_acc_0.adjusted(s2, total_lq_2, events_2);
        assert_eq!(
            updated_acc_2,
            AccountPosition {
                avg_share_bps: 2500,
                share: (1000000, 4000000,),
                updated_at: 30,
                created_at: 10,
            },
        );
    }
}
