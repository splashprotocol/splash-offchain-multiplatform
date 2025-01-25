use crate::event::LpEvent;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Account {
    weight: u64,
}

impl Account {
    pub fn empty() -> Self {
        Self { weight: 0 }
    }
    pub fn apply_event(self, ev: LpEvent) -> Self {
        todo!()
    }
}
