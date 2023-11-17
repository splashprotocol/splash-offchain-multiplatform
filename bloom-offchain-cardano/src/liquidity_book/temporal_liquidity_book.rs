use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::collections::btree_map::Entry;
use std::mem;

use crate::liquidity_book::liquidity::{LiquidityFragment, PooledLiquidity, OneSideLiquidity};
use crate::liquidity_book::LiquidityBook;
use crate::liquidity_book::market_effect::MarketEffect;
use crate::liquidity_book::match_candidates::MatchCandidates;
use crate::liquidity_book::types::SourceId;

#[derive(Debug, Clone)]
pub struct Fragments<T> {
    asks: BTreeSet<LiquidityFragment<T>>,
    bids: BTreeSet<LiquidityFragment<T>>,
}

impl<T> Fragments<T> {
    fn new() -> Self {
        Self {
            asks: BTreeSet::new(),
            bids: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FragmentedLiquidity<T> {
    /// Liquidity fragments spread across time axis.
    /// First element in the tuple encodes the time point which is now.
    chronology: (Fragments<T>, BTreeMap<T, Fragments<T>>),
    known_fragments: HashSet<SourceId>,
    removed_fragments: HashSet<SourceId>,
}

impl<T> FragmentedLiquidity<T> where T: Ord + Copy {

    fn advance_clocks(&mut self, new_time: T) {
        let new_slot = self.chronology.1.remove(&new_time).unwrap_or_else(|| Fragments::new());
        let Fragments { asks, bids } = mem::replace(&mut self.chronology.0, new_slot);
        for fr in asks {
            if fr.bounds.contains(&new_time) { self.chronology.0.asks.insert(fr); }
        }
        for fr in bids {
            if fr.bounds.contains(&new_time) { self.chronology.0.bids.insert(fr); }
        }
    }

    fn remove_fragment(&mut self, source: SourceId) {
        if self.known_fragments.remove(&source) {
            self.removed_fragments.insert(source);
        }
    }

    fn add_fragment(&mut self, fr: OneSideLiquidity<LiquidityFragment<T>>) {
        let any_side_fr = fr.any();
        // Clear removal filter just in case the fragment was re-added.
        self.removed_fragments.remove(&any_side_fr.id);
        self.known_fragments.insert(any_side_fr.id);
        if let Some(initial_timeslot) = any_side_fr.bounds.lower_bound() {
            match self.chronology.1.entry(initial_timeslot) {
                Entry::Vacant(e) => {
                    let mut fresh_fragments = Fragments::new();
                    match fr {
                        OneSideLiquidity::Bid(fr) => fresh_fragments.bids.insert(fr),
                        OneSideLiquidity::Ask(fr) => fresh_fragments.asks.insert(fr),
                    };
                    e.insert(fresh_fragments);
                },
                Entry::Occupied(e) => {
                    match fr {
                        OneSideLiquidity::Bid(fr) => e.into_mut().bids.insert(fr),
                        OneSideLiquidity::Ask(fr) => e.into_mut().asks.insert(fr),
                    };
                },
            }
        }
    }
}

/// Aggregates composable liquidity in three dimensions: price, volume, time.
pub struct TemporalLiquidityBook<T> {
    fragmented_liquidity: FragmentedLiquidity<T>,
    pooled_liquidity: HashSet<PooledLiquidity>,
}

impl<T> TemporalLiquidityBook<T> {
}

impl<T> LiquidityBook<MarketEffect<T>> for TemporalLiquidityBook<T> {
    fn apply_effect(&mut self, effects: Vec<MarketEffect<T>>) {
        todo!()
    }
    fn pull_candidates(&self) -> Option<MatchCandidates> {
        todo!()
    }
}
