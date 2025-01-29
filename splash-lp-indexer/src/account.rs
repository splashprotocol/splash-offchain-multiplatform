use crate::event::LpEvent;
use cml_core::Slot;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct AccountInPool {
    avg_share_bps: u64,
    share: (u64, u64),
    updated_at: Slot,
}

impl AccountInPool {
    pub fn new() -> Self {
        Self {
            avg_share_bps: 0,
            share: (0, 1),
            updated_at: 0,
        }
    }

    // share_bps{acc,pool,slot} = lq{acc,pool,slot} * 100^2 / total_lq{pool,slot}
    pub fn apply_events(
        self,
        genesis_slot: Slot,
        current_slot: Slot,
        total_lq: u64,
        events: Vec<LpEvent>,
    ) -> Self {
        // 1. compute avg_share_bps for previous slots
        todo!()
    }
}
