use crate::onchain::event::PositionEvent;
use cml_core::Slot;
use log::info;
use serde::{Deserialize, Serialize};
use splash_yf_offchain::Epoch;

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Debug)]
pub struct AccountPosition {
    pub share_intervals: Vec<ShareInterval>,
}

impl AccountPosition {
    pub fn new(current_slot: Slot, share: (u64, u64)) -> Self {
        Self {
            share_intervals: vec![ShareInterval::new(current_slot, current_slot, share)],
        }
    }

    pub fn is_currently_zero_share(&self) -> bool {
        self.share_intervals.is_empty() || self.share_intervals.last().unwrap().share.0 == 0
    }

    pub fn weighted_average_share_bps(&self) -> u64 {
        if self.share_intervals.is_empty() {
            return 0;
        }
        let start_slot = self.share_intervals.first().unwrap().start;
        let last_interval = self.share_intervals.last().unwrap();
        let end_slot = last_interval.end;
        if end_slot == start_slot {
            return 0;
        }

        self.share_intervals
            .iter()
            .map(|interval| interval.share_bps())
            .sum::<u64>()
            .checked_div(end_slot - start_slot)
            .unwrap()
    }

    /// Update position in response to external pool changes (i.e. other users depositing or redeeming).
    pub fn update_from_external_pool_changes<E>(
        &mut self,
        current_slot: Slot,
        new_pool_lp_supply: u64,
        converter: &E,
    ) where
        E: EpochSlotConversion,
    {
        if let Some(current_position) = self.share_intervals.last().map(|interval| interval.share.0) {
            if current_position > 0 {
                self.add_new_share(current_slot, (current_position, new_pool_lp_supply), converter);
            }
        }
    }

    /// Update the account position from user event (deposit or redeem).
    pub fn update_from_user_event<E>(&mut self, current_slot: Slot, event: PositionEvent, converter: &E)
    where
        E: EpochSlotConversion,
    {
        if let Some((personal_position_lq, total_lq)) =
            self.get_current_share().map(|interval| interval.share)
        {
            match event {
                PositionEvent::Deposit(deposit) => {
                    let new_position_lq = personal_position_lq.checked_add(deposit.lp_mint).unwrap();
                    if total_lq.checked_add(deposit.lp_mint).unwrap() == deposit.lp_supply {
                        info!("DDD: deposit.lp_supply == total_lq + deposit.lp_mint");
                    } else if total_lq == deposit.lp_supply {
                        info!("EEE: deposit.lp_supply == total_lq");
                    }
                    self.add_new_share(current_slot, (new_position_lq, deposit.lp_supply), converter);
                }
                PositionEvent::Redeem(redeem) => {
                    let new_position_lq = personal_position_lq.checked_sub(redeem.lp_burned).unwrap();
                    // assert_eq!(total_lq, redeem.lp_supply);
                    self.add_new_share(current_slot, (new_position_lq, redeem.lp_supply), converter);
                }
            }
        } else {
            match event {
                PositionEvent::Deposit(deposit) => {
                    self.add_new_share(current_slot, (deposit.lp_mint, deposit.lp_supply), converter);
                }
                PositionEvent::Redeem(_) => {
                    unreachable!("Cannot redeem from an empty position");
                }
            }
        }
    }

    pub fn get_current_share(&self) -> Option<ShareInterval> {
        self.share_intervals.last().cloned()
    }

    pub fn rollback_to(&mut self, current_slot: Slot) {
        if let Some(created_at_slot) = self.share_intervals.first().map(|interval| interval.start) {
            if created_at_slot > current_slot {
                self.share_intervals.clear();
                return;
            }
        }
        self.share_intervals.retain(|interval| {
            interval.end <= current_slot || (interval.start <= current_slot && interval.end > current_slot)
        });
        if let Some(last_interval) = self.share_intervals.last_mut() {
            if last_interval.end > current_slot {
                assert!(last_interval.start <= current_slot);
                last_interval.end = current_slot;
            }
        }
    }

    fn add_new_share<E>(&mut self, current_slot: Slot, share: (u64, u64), converter: &E)
    where
        E: EpochSlotConversion,
    {
        if let Some(last_interval) = self.share_intervals.last_mut() {
            assert!(last_interval.end <= current_slot);
            let last_interval_epoch = converter.to_epoch(last_interval.end);
            if last_interval_epoch == converter.to_epoch(current_slot) {
                last_interval.extend_to(current_slot);
            } else {
                last_interval.extend_to(converter.last_slot(last_interval_epoch));
            }
        }
        // Clear out all intervals with empty weight
        self.share_intervals
            .retain(|interval| interval.start < interval.end);

        self.share_intervals
            .push(ShareInterval::new(current_slot, current_slot, share));
    }

    pub fn extend_current_share_to(&mut self, current_slot: Slot) {
        self.share_intervals.last_mut().unwrap().extend_to(current_slot);
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Serialize, Deserialize, Debug)]
pub struct ShareInterval {
    pub start: Slot,
    pub end: Slot,
    pub share: (u64, u64),
}

impl ShareInterval {
    pub fn new(start: Slot, end: Slot, share: (u64, u64)) -> Self {
        Self { start, end, share }
    }

    fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    pub fn extend_to(&mut self, current_slot: Slot) {
        self.end = current_slot;
    }

    pub fn share_bps(&self) -> u64 {
        let res = (self.end - self.start) * (self.share.0 * 10_000 / self.share.1);
        info!(
            "share_bps: {}, start: {}, end: {}, share: {:?}",
            res, self.start, self.end, self.share
        );
        res
    }
}

pub trait EpochSlotConversion {
    fn to_epoch(&self, slot: Slot) -> Epoch;
    fn first_slot(&self, epoch: Epoch) -> Slot;
    fn last_slot(&self, epoch: Epoch) -> Slot;
}

pub struct DefaultEpochSlotConversion {
    slots_in_epoch: u64,
    epoch_start: Slot,
}

impl DefaultEpochSlotConversion {
    pub fn new(slots_in_epoch: u64, epoch_start: Slot) -> Self {
        Self {
            slots_in_epoch,
            epoch_start,
        }
    }
}

impl EpochSlotConversion for DefaultEpochSlotConversion {
    fn to_epoch(&self, slot: Slot) -> Epoch {
        Epoch::unsafe_from_slot(slot, self.slots_in_epoch, self.epoch_start)
    }

    fn first_slot(&self, epoch: Epoch) -> Slot {
        epoch.first_slot(self.slots_in_epoch, self.epoch_start)
    }

    fn last_slot(&self, epoch: Epoch) -> Slot {
        epoch.last_slot(self.slots_in_epoch, self.epoch_start)
    }
}
#[cfg(test)]
mod tests {
    use crate::account::{AccountPosition, DefaultEpochSlotConversion};
    use crate::onchain::event::{Deposit, PositionEvent, Redeem};
    use cml_chain::certs::Credential;
    use cml_crypto::Ed25519KeyHash;
    use spectrum_offchain_cardano::data::PoolId;

    #[test]
    fn deposit_redeem_rollback() {
        let s0 = 10;
        let account_key = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));
        let personal_position_lq = 50_000;
        let total_lq = 1_000_000;
        let pool_id = PoolId::random();
        let mut acc = AccountPosition::new(s0, (personal_position_lq, total_lq));

        let s1 = s0 + 10;

        let last_interval = acc.share_intervals.last().unwrap();
        assert_eq!(last_interval.share, (personal_position_lq, total_lq));
        assert_eq!(acc.weighted_average_share_bps(), 0);

        acc.extend_current_share_to(s1);

        let first_interval_bps = (s1 - s0) * personal_position_lq * 10_000 / total_lq;
        assert_eq!(acc.weighted_average_share_bps(), first_interval_bps / 10);

        let s2 = s1 + 60;
        let personal_position_lq_2 = personal_position_lq * 2;
        let total_lq_2 = total_lq + personal_position_lq;
        let event = PositionEvent::Deposit(Deposit {
            pool_id,
            account: account_key.clone(),
            lp_mint: personal_position_lq,
            lp_supply: total_lq_2,
        });
        let converter = DefaultEpochSlotConversion::new(1000, 0);
        acc.update_from_user_event(s2, event, &converter);
        assert_eq!(acc.weighted_average_share_bps(), first_interval_bps / 10);

        let s3 = s2 + 80;
        acc.extend_current_share_to(s3);
        let second_interval_bps = (s3 - s2) * personal_position_lq_2 * 10_000 / total_lq_2;
        let bps_s3 = (second_interval_bps + (s2 - s0) * first_interval_bps / (s1 - s0)) / (s3 - s0);
        assert_eq!(acc.weighted_average_share_bps(), bps_s3);

        // Extend current share by 1 slot
        let s4 = s3 + 1;
        let bps_s4 = (second_interval_bps / (s3 - s2) * (s4 - s2)
            + (s2 - s0) * first_interval_bps / (s1 - s0))
            / (s4 - s0);
        acc.extend_current_share_to(s4);
        assert_eq!(acc.weighted_average_share_bps(), bps_s4);

        // Redeem entire position
        let s5 = s4 + 100;
        let event = PositionEvent::Redeem(Redeem {
            pool_id,
            account: account_key,
            lp_burned: 2 * personal_position_lq,
            lp_supply: total_lq,
        });
        acc.update_from_user_event(s5, event, &converter);
        assert!(acc.is_currently_zero_share());
        let bps_s5 = (second_interval_bps / (s3 - s2) * (s5 - s2)
            + (s2 - s0) * first_interval_bps / (s1 - s0))
            / (s5 - s0);
        assert_eq!(acc.weighted_average_share_bps(), bps_s5);

        // Rollback to s4
        acc.rollback_to(s4);
        assert!(!acc.is_currently_zero_share());
        assert_eq!(acc.weighted_average_share_bps(), bps_s4);

        // Rollback to s3
        acc.rollback_to(s3);
        assert_eq!(acc.weighted_average_share_bps(), bps_s3);

        // Rollback prior to s0, so that the position should no longer exist.
        acc.rollback_to(s0 - 1);
        assert!(acc.is_currently_zero_share());
        assert_eq!(acc.weighted_average_share_bps(), 0);
    }

    #[test]
    fn external_pool_changes() {
        let s0 = 10;
        let personal_position_lq = 50_000;
        let total_lq = 1_000_000;
        let converter = DefaultEpochSlotConversion::new(1000, 0);
        let mut acc = AccountPosition::new(s0, (personal_position_lq, total_lq));

        let s1 = s0 + 10;
        acc.update_from_external_pool_changes(s1, 2 * total_lq, &converter);

        let first_interval_bps = (s1 - s0) * personal_position_lq * 10_000 / total_lq;
        assert_eq!(acc.weighted_average_share_bps(), first_interval_bps / 10);

        let s2 = s1 + 60;
        acc.extend_current_share_to(s2);
        let bps_s2 =
            (first_interval_bps + (s2 - s1) * personal_position_lq * 10_000 / total_lq / 2) / (s2 - s0);
        assert_eq!(acc.weighted_average_share_bps(), bps_s2);
    }
}
