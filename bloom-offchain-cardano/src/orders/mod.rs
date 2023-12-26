use cml_chain::PolicyId;
use cml_multi_era::babbage::BabbageTransactionOutput;

use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use spectrum_cardano_lib::{NetworkTime, OutputRef};
use spectrum_offchain::data::EntitySnapshot;
use spectrum_offchain::ledger::TryFromLedger;

pub mod auction;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum AnyOrder {}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for AnyOrder {
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        todo!()
    }
}

impl EntitySnapshot for AnyOrder {
    type Version = OutputRef;
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        todo!()
    }
    fn version(&self) -> Self::Version {
        todo!()
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Stateful<O, S> {
    pub order: O,
    pub state: S,
}

impl<O, S> Stateful<O, S> {
    pub fn new(order: O, state: S) -> Self {
        Self { order, state }
    }
}

/// State of order compatible with TLB.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct TLBCompatibleState {
    /// Side of the order relative to pair it maps to.
    pub side: SideM,
    pub time_now: NetworkTime,
}
