use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::collections::btree_map::Entry;
use std::mem;

use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use crate::execution_engine::liquidity_book::side::{Side, SideMarker};
use crate::execution_engine::liquidity_book::types::Price;
use crate::execution_engine::SourceId;

pub trait FragmentedLiquidity<Fr> {
    fn best_price(&self, side: SideMarker) -> Option<Side<Price>>;
    fn pick_either(&mut self) -> Option<Side<Fr>>;
    fn try_pick<F>(&mut self, side: SideMarker, test: F) -> Option<Fr>
    where
        F: FnOnce(&Fr) -> bool;
    fn return_fr(&mut self, fr: Side<Fr>);
}

pub trait FragmentStore<Fr> {
    fn advance_clocks(&mut self, new_time: u64);
    fn remove_fragments(&mut self, source: SourceId);
    fn add_fragment(&mut self, fr: Side<Fr>);
}

/// Liquidity fragments spread across time axis.
#[derive(Debug, Clone)]
struct Chronology<Fr> {
    time_now: u64,
    now: Fragments<Fr>,
    later: BTreeMap<u64, Fragments<Fr>>,
}

#[derive(Debug, Clone)]
pub struct InMemoryFragmentedLiquidity<Fr> {
    chronology: Chronology<Fr>,
    index: HashMap<SourceId, Side<Fr>>,
}

impl<Fr> FragmentedLiquidity<Fr> for InMemoryFragmentedLiquidity<Fr>
where
    Fr: Fragment + Ord,
{
    fn best_price(&self, side: SideMarker) -> Option<Side<Price>> {
        let side_store = match side {
            SideMarker::Bid => &self.chronology.now.bids,
            SideMarker::Ask => &self.chronology.now.asks,
        };
        side_store.first().map(|fr| side.wrap(fr.price()))
    }

    fn pick_either(&mut self) -> Option<Side<Fr>> {
        let best_bid = self.chronology.now.bids.pop_first();
        let best_ask = self.chronology.now.asks.pop_first();
        match (best_bid, best_ask) {
            (Some(bid), Some(ask)) if bid.weight() >= ask.weight() => Some(Side::Bid(bid)),
            (Some(_), Some(ask)) => Some(Side::Ask(ask)),
            (Some(bid), None) => Some(Side::Bid(bid)),
            (None, Some(ask)) => Some(Side::Ask(ask)),
            _ => None,
        }
    }

    fn try_pick<F>(&mut self, side: SideMarker, test: F) -> Option<Fr>
    where
        F: FnOnce(&Fr) -> bool,
    {
        let side = match side {
            SideMarker::Bid => &mut self.chronology.now.bids,
            SideMarker::Ask => &mut self.chronology.now.asks,
        };
        side.pop_first()
            .and_then(|best_bid| if test(&best_bid) { Some(best_bid) } else { None })
    }

    fn return_fr(&mut self, fr: Side<Fr>) {
        match fr {
            Side::Bid(bid) => self.chronology.now.bids.insert(bid),
            Side::Ask(ask) => self.chronology.now.bids.insert(ask),
        };
    }
}

impl<Fr> FragmentStore<Fr> for InMemoryFragmentedLiquidity<Fr>
where
    Fr: Fragment + OrderState + Copy + Ord,
{
    fn advance_clocks(&mut self, new_time: u64) {
        let new_slot = self
            .chronology
            .later
            .remove(&new_time)
            .unwrap_or_else(|| Fragments::new());
        let Fragments { asks, bids } = mem::replace(&mut self.chronology.now, new_slot);
        for fr in asks {
            if let StateTrans::Active(next_fr) = fr.with_updated_time(new_time) {
                self.chronology.now.asks.insert(next_fr);
            }
        }
        for fr in bids {
            if let StateTrans::Active(next_fr) = fr.with_updated_time(new_time) {
                self.chronology.now.bids.insert(next_fr);
            }
        }
        self.chronology.time_now = new_time;
    }

    fn remove_fragments(&mut self, source: SourceId) {
        if let Some(fr) = self.index.remove(&source) {
            if let Some(initial_timeslot) = fr.any().time_bounds().lower_bound() {
                if initial_timeslot <= self.chronology.time_now {
                    match fr {
                        Side::Bid(fr) => self.chronology.now.bids.remove(&fr),
                        Side::Ask(fr) => self.chronology.now.asks.remove(&fr),
                    };
                } else {
                    match self.chronology.later.entry(initial_timeslot) {
                        Entry::Occupied(e) => {
                            match fr {
                                Side::Bid(fr) => e.into_mut().bids.remove(&fr),
                                Side::Ask(fr) => e.into_mut().asks.remove(&fr),
                            };
                        }
                        Entry::Vacant(_) => {}
                    }
                }
            }
        }
    }

    fn add_fragment(&mut self, fr: Side<Fr>) {
        self.index.insert(fr.any().source(), fr);
        if let Some(initial_timeslot) = fr.any().time_bounds().lower_bound() {
            if initial_timeslot <= self.chronology.time_now {
                match fr {
                    Side::Bid(fr) => self.chronology.now.bids.insert(fr),
                    Side::Ask(fr) => self.chronology.now.asks.insert(fr),
                };
            } else {
                match self.chronology.later.entry(initial_timeslot) {
                    Entry::Vacant(e) => {
                        let mut fresh_fragments = Fragments::new();
                        match fr {
                            Side::Bid(fr) => fresh_fragments.bids.insert(fr),
                            Side::Ask(fr) => fresh_fragments.asks.insert(fr),
                        };
                        e.insert(fresh_fragments);
                    }
                    Entry::Occupied(e) => {
                        match fr {
                            Side::Bid(fr) => e.into_mut().bids.insert(fr),
                            Side::Ask(fr) => e.into_mut().asks.insert(fr),
                        };
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Fragments<Fr> {
    asks: BTreeSet<Fr>,
    bids: BTreeSet<Fr>,
}

impl<Fr> Fragments<Fr> {
    fn new() -> Self {
        Self {
            asks: BTreeSet::new(),
            bids: BTreeSet::new(),
        }
    }
}
