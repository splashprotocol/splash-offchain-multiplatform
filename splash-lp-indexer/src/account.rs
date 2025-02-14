use crate::onchain::event::{Harvest, PositionEvent, SuspendedPositionEvents};
use cml_core::Slot;
use serde::{Deserialize, Serialize};

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Debug)]
pub struct AccountInPool {
    /// Accumulator of avg share over period from `created_at` to `activated_at`
    avg_share_bps: u64,
    /// Latest share as (personal_share, total_share)
    share: (u64, u64),
    updated_at: Slot,
    activated_at: Option<Slot>,
    pub locked_at: Option<Slot>,
}

impl AccountInPool {
    pub fn new(current_slot: Slot, activated: bool) -> Self {
        Self {
            avg_share_bps: 0,
            share: (0, 1),
            updated_at: current_slot,
            activated_at: if activated { Some(current_slot) } else { None },
            locked_at: None,
        }
    }

    pub fn activated(mut self, slot: Slot) -> Self {
        self.activated_at.replace(slot);
        self
    }

    pub fn deactivated(mut self) -> Self {
        self.activated_at = None;
        self
    }

    // Lock account before harvesting.
    pub fn lock(mut self, current_slot: Slot) -> Self {
        self.locked_at.replace(current_slot);
        self
    }

    // Unlock if harvest failed.
    pub fn unlock(mut self, delayed_events: Vec<SuspendedPositionEvents>) -> Self {
        self.locked_at = None;
        delayed_events.into_iter().fold(self, |acc, de| {
            acc.try_adjust_position(de.current_slot, de.total_lq, de.events)
                .unwrap()
        })
    }

    // Process harvest event if harvest succeeded.
    pub fn harvest(mut self, delayed_events: Vec<SuspendedPositionEvents>, harvest: Harvest) -> Self {
        if let Some(ref mut activated_at) = self.activated_at {
            *activated_at = harvest.harvested_till;
        };
        self.unlock(delayed_events)
    }

    pub fn try_adjust_position(
        mut self,
        current_slot: Slot,
        total_lq: u64,
        events: Vec<PositionEvent>,
    ) -> Result<Self, (Self, SuspendedPositionEvents)> {
        if self.locked_at.is_some() {
            return Err((
                self,
                SuspendedPositionEvents {
                    current_slot,
                    total_lq,
                    events,
                },
            ));
        }
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
        Ok(self)
    }

    fn share_bps(&self) -> u64 {
        self.share.0 * 10_000 / self.share.1
    }
}

#[cfg(test)]
mod tests {
    use crate::account::AccountInPool;
    use crate::onchain::event::{Deposit, PositionEvent, Redeem};
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
        let init_acc = acc.try_adjust_position(s0, total_lq, events);
        assert_eq!(
            init_acc,
            Ok(AccountInPool {
                avg_share_bps: 0,
                share: (personal_position_lq, total_lq),
                updated_at: s0,
                activated_at: Some(s0),
                locked_at: None,
            })
        )
    }


    #[test]
    fn deposit_redeem() {
        let s0 = 10;
        let s1 = 20;
        let s2 = 30;
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
        let init_acc = acc.try_adjust_position(s0, total_lq_0, events_0).unwrap();
        let personal_delta_lq_1 = 500_000;
        let total_lq_1 = 4_000_000;
        let events_1 = vec![PositionEvent::Redeem(Redeem {
            pool_id: pid,
            account: account_key.clone(),
            lp_burned: personal_delta_lq_1,
            lp_supply: total_lq_1,
        })];
        let updated_acc_0 = init_acc.try_adjust_position(s1, total_lq_1, events_1).unwrap();
        let personal_delta_lq_2 = 1_000_000;
        let total_lq_2 = 4_000_000;
        let events_2 = vec![PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account_key,
            lp_mint: personal_delta_lq_2,
            lp_supply: total_lq_2,
        })];
        let updated_acc_2 = updated_acc_0.try_adjust_position(s2, total_lq_2, events_2);
        assert_eq!(updated_acc_2, Ok(
            AccountInPool {
                avg_share_bps: 2500,
                share: (
                    1000000,
                    4000000,
                ),
                updated_at: 30,
                activated_at: Some(
                    10,
                ),
                locked_at: None,
            },
        ));
    }
}
