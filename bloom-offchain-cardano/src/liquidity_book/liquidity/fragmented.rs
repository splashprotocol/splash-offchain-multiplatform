use std::collections::{BTreeMap, BTreeSet, hash_map, HashMap};
use std::collections::btree_map::Entry;
use std::hash::Hash;
use std::mem;

use crate::liquidity_book::fragment::Fragment;
use crate::liquidity_book::side::{Side, SideMarker};
use crate::liquidity_book::types::{Price, SourceId};

pub trait FragmentedLiquidity<T, Fr> {
    fn best_price(&self, side: SideMarker) -> Option<Side<Price>>;
    fn pick_either(&mut self) -> Option<Side<Fr>>;
    fn try_pick<F>(&mut self, side: SideMarker, test: F) -> Option<Fr>
    where
        F: FnOnce(&Fr) -> bool;
}

pub trait FragmentStore<T, Fr> {
    fn advance_clocks(&mut self, new_time: T);
    fn remove_fragments(&mut self, source: SourceId);
    fn add_fragments(&mut self, source_id: SourceId, fr: Vec<Side<Fragment<T>>>);
}

#[derive(Debug, Clone)]
struct Chronology<T, Fr> {
    time_now: T,
    now: Fragments<Fr>,
    later: BTreeMap<T, Fragments<Fr>>,
}

#[derive(Debug, Clone)]
pub struct InMemoryFragmentedLiquidity<T, Fr> {
    /// Liquidity fragments spread across time axis.
    /// First element in the tuple encodes the time point which is now.
    chronology: Chronology<T, Fr>,
    sources: HashMap<SourceId, Vec<Side<Fr>>>,
}

impl<T> FragmentedLiquidity<T, Fragment<T>> for InMemoryFragmentedLiquidity<T, Fragment<T>>
where
    T: Ord,
{
    fn best_price(&self, side: SideMarker) -> Option<Side<Price>> {
        let side_store = match side {
            SideMarker::Bid => &self.chronology.now.bids,
            SideMarker::Ask => &self.chronology.now.asks,
        };
        side_store.first().map(|fr| side.wrap(fr.price))
    }

    fn pick_either(&mut self) -> Option<Side<Fragment<T>>> {
        let best_bid = self.chronology.now.bids.pop_first();
        let best_ask = self.chronology.now.asks.pop_first();
        match (best_bid, best_ask) {
            (Some(bid), Some(ask)) if bid.fee >= ask.fee => Some(Side::Bid(bid)),
            (Some(_), Some(ask)) => Some(Side::Ask(ask)),
            (Some(bid), None) => Some(Side::Bid(bid)),
            (None, Some(ask)) => Some(Side::Ask(ask)),
            _ => None,
        }
    }

    fn try_pick<F>(&mut self, side: SideMarker, test: F) -> Option<Fragment<T>>
    where
        F: FnOnce(&Fragment<T>) -> bool,
    {
        let side = match side {
            SideMarker::Bid => &mut self.chronology.now.bids,
            SideMarker::Ask => &mut self.chronology.now.asks,
        };
        side.pop_first()
            .and_then(|best_bid| if test(&best_bid) { Some(best_bid) } else { None })
    }
}

impl<T> FragmentStore<T, Fragment<T>> for InMemoryFragmentedLiquidity<T, Fragment<T>>
where
    T: Ord + Copy + Eq + Hash,
{
    fn advance_clocks(&mut self, new_time: T) {
        let new_slot = self
            .chronology
            .later
            .remove(&new_time)
            .unwrap_or_else(|| Fragments::new());
        let Fragments { asks, bids } = mem::replace(&mut self.chronology.now, new_slot);
        for fr in asks {
            if fr.bounds.contain(&new_time) {
                self.chronology.now.asks.insert(fr);
            }
        }
        for fr in bids {
            if fr.bounds.contain(&new_time) {
                self.chronology.now.bids.insert(fr);
            }
        }
        self.chronology.time_now = new_time;
    }

    fn remove_fragments(&mut self, source: SourceId) {
        match self.sources.entry(source) {
            hash_map::Entry::Occupied(occupied) => {
                let frs = occupied.remove();
                for fr in frs {
                    match fr {
                        Side::Bid(fr) => {
                            if let Some(lb) = fr.bounds.lower_bound() {
                                if lb > self.chronology.time_now {
                                    if let Some(slot) = self.chronology.later.get_mut(&lb) {
                                        slot.bids.remove(&fr);
                                    }
                                } else {
                                    self.chronology.now.bids.remove(&fr);
                                }
                            }
                        }
                        Side::Ask(fr) => {
                            if let Some(lb) = fr.bounds.lower_bound() {
                                if lb > self.chronology.time_now {
                                    if let Some(slot) = self.chronology.later.get_mut(&lb) {
                                        slot.asks.remove(&fr);
                                    }
                                } else {
                                    self.chronology.now.asks.remove(&fr);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn add_fragments(&mut self, source: SourceId, frs: Vec<Side<Fragment<T>>>) {
        for fr in &frs {
            let any_side_fr = fr.any();
            if let Some(initial_timeslot) = any_side_fr.bounds.lower_bound() {
                match self.chronology.later.entry(initial_timeslot) {
                    Entry::Vacant(e) => {
                        let mut fresh_fragments = Fragments::new();
                        match fr {
                            Side::Bid(fr) => fresh_fragments.bids.insert(*fr),
                            Side::Ask(fr) => fresh_fragments.asks.insert(*fr),
                        };
                        e.insert(fresh_fragments);
                    }
                    Entry::Occupied(e) => {
                        match fr {
                            Side::Bid(fr) => e.into_mut().bids.insert(*fr),
                            Side::Ask(fr) => e.into_mut().asks.insert(*fr),
                        };
                    }
                }
            }
        }
        self.sources.insert(source, frs);
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
