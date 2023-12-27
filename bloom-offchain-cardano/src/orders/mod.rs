use cml_chain::PolicyId;
use cml_multi_era::babbage::BabbageTransactionOutput;

use bloom_offchain::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{BasePrice, ExecutionCost};
use spectrum_cardano_lib::{NetworkTime, OutputRef};
use spectrum_offchain::data::{EntitySnapshot, Tradable};
use spectrum_offchain::ledger::TryFromLedger;

use crate::PairId;

pub mod auction;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub enum AnyOrder {}

impl OrderState for AnyOrder {
    fn with_updated_liquidity(self, removed_input: u64, added_output: u64) -> StateTrans<Self> {
        todo!()
    }
    fn with_updated_time(self, time: u64) -> StateTrans<Self> {
        todo!()
    }
}

impl Fragment for AnyOrder {
    fn side(&self) -> SideM {
        todo!()
    }

    fn input(&self) -> u64 {
        todo!()
    }

    fn price(&self) -> BasePrice {
        todo!()
    }

    fn weight(&self) -> u64 {
        todo!()
    }

    fn cost_hint(&self) -> ExecutionCost {
        todo!()
    }

    fn time_bounds(&self) -> TimeBounds<u64> {
        todo!()
    }
}

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

impl Tradable for AnyOrder {
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
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
