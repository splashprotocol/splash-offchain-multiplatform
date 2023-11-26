use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::collections::btree_map::Entry;
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
    fn remove_fragment(&mut self, source: SourceId);
    fn add_fragment(&mut self, fr: Side<Fragment<T>>);
}

#[derive(Debug, Clone)]
pub struct InMemoryFragmentedLiquidity<T, Fr> {
    /// Liquidity fragments spread across time axis.
    /// First element in the tuple encodes the time point which is now.
    chronology: (Fragments<Fr>, BTreeMap<T, Fragments<Fr>>),
    known_fragments: HashSet<SourceId>,
    removed_fragments: HashSet<SourceId>,
}

impl<T> FragmentedLiquidity<T, Fragment<T>> for InMemoryFragmentedLiquidity<T, Fragment<T>>
where
    T: Ord,
{
    fn best_price(&self, side: SideMarker) -> Option<Side<Price>> {
        let side_store = match side {
            SideMarker::Bid => &self.chronology.0.bids,
            SideMarker::Ask => &self.chronology.0.asks,
        };
        side_store.first().map(|fr| side.wrap(fr.price))
    }

    fn pick_either(&mut self) -> Option<Side<Fragment<T>>> {
        let best_bid = self.chronology.0.bids.pop_first();
        let best_ask = self.chronology.0.asks.pop_first();
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
            SideMarker::Bid => &mut self.chronology.0.bids,
            SideMarker::Ask => &mut self.chronology.0.asks,
        };
        side.pop_first()
            .and_then(|best_bid| if test(&best_bid) { Some(best_bid) } else { None })
    }
}

impl<T> InMemoryFragmentedLiquidity<T, Fragment<T>>
where
    T: Ord + Copy,
{
    fn try_pop_best<F>(&mut self, side: SideMarker, test: F) -> Option<Fragment<T>>
    where
        F: FnOnce(&Fragment<T>) -> bool,
    {
        let side = match side {
            SideMarker::Bid => &mut self.chronology.0.bids,
            SideMarker::Ask => &mut self.chronology.0.asks,
        };
        side.pop_first()
            .and_then(|best_bid| if test(&best_bid) { Some(best_bid) } else { None })
    }

    fn advance_clocks(&mut self, new_time: T) {
        let new_slot = self
            .chronology
            .1
            .remove(&new_time)
            .unwrap_or_else(|| Fragments::new());
        let Fragments { asks, bids } = mem::replace(&mut self.chronology.0, new_slot);
        for fr in asks {
            if fr.bounds.contain(&new_time) {
                self.chronology.0.asks.insert(fr);
            }
        }
        for fr in bids {
            if fr.bounds.contain(&new_time) {
                self.chronology.0.bids.insert(fr);
            }
        }
    }

    fn remove_fragment(&mut self, source: SourceId) {
        if self.known_fragments.remove(&source) {
            self.removed_fragments.insert(source);
        }
    }

    fn add_fragment(&mut self, fr: Side<Fragment<T>>) {
        let any_side_fr = fr.any();
        // Clear removal filter just in case the fragment was re-added.
        self.removed_fragments.remove(&any_side_fr.source);
        self.known_fragments.insert(any_side_fr.source);
        if let Some(initial_timeslot) = any_side_fr.bounds.lower_bound() {
            match self.chronology.1.entry(initial_timeslot) {
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
