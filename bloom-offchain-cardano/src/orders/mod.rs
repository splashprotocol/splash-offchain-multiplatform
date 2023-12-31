use cml_multi_era::babbage::BabbageTransactionOutput;

use bloom_derivation::{EntitySnapshot, Fragment};
use bloom_offchain::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use spectrum_cardano_lib::{NetworkTime, OutputRef};
use spectrum_offchain::data::Tradable;
use spectrum_offchain::ledger::TryFromLedger;

use crate::orders::spot::SpotOrder;
use crate::PairId;

pub mod auction;
pub mod spot;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Fragment, EntitySnapshot)]
pub enum AnyOrder {
    Spot(SpotOrder),
}

impl OrderState for AnyOrder {
    fn with_updated_time(self, time: u64) -> StateTrans<Self> {
        match self {
            AnyOrder::Spot(spot) => spot.with_updated_time(time).map(AnyOrder::Spot),
        }
    }
    fn with_updated_liquidity(self, removed_input: u64, added_output: u64) -> StateTrans<Self> {
        match self {
            AnyOrder::Spot(spot) => spot
                .with_updated_liquidity(removed_input, added_output)
                .map(AnyOrder::Spot),
        }
    }
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for AnyOrder {
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
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
