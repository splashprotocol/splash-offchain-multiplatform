use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::collections::btree_map::Entry;
use std::mem;

use cml_core::Slot;
use futures::future::Either;

use spectrum_cardano_lib::TaggedAmount;

use crate::liquidity_book::liquidity::{
    AnyFragment, AnyPool, LiquidityFragment, LiquidityPool, OneSideLiquidity,
};
use crate::liquidity_book::LiquidityBook;
use crate::liquidity_book::market_effect::MarketEffect;
use crate::liquidity_book::match_candidates::MatchCandidates;
use crate::liquidity_book::types::{Base, Price, Side, SourceId};

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

#[derive(Debug, Clone)]
pub struct FragmentedLiquidity<T, Fr> {
    /// Liquidity fragments spread across time axis.
    /// First element in the tuple encodes the time point which is now.
    chronology: (Fragments<Fr>, BTreeMap<T, Fragments<Fr>>),
    known_fragments: HashSet<SourceId>,
    removed_fragments: HashSet<SourceId>,
}

impl<T, Fr> FragmentedLiquidity<T, Fr>
where
    T: Ord + Copy,
    Fr: LiquidityFragment<T>,
{
    fn try_pop_best<F>(&mut self, side: Side, test: F) -> Option<Fr>
    where
        F: FnOnce(&Fr) -> bool,
    {
        let side = match side {
            Side::Bid => &mut self.chronology.0.bids,
            Side::Ask => &mut self.chronology.0.asks,
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
            if fr.time_bounds().contain(&new_time) {
                self.chronology.0.asks.insert(fr);
            }
        }
        for fr in bids {
            if fr.time_bounds().contain(&new_time) {
                self.chronology.0.bids.insert(fr);
            }
        }
    }

    fn remove_fragment(&mut self, source: SourceId) {
        if self.known_fragments.remove(&source) {
            self.removed_fragments.insert(source);
        }
    }

    fn add_fragment(&mut self, fr: OneSideLiquidity<Fr>) {
        let any_side_fr = fr.any();
        // Clear removal filter just in case the fragment was re-added.
        self.removed_fragments.remove(&any_side_fr.id());
        self.known_fragments.insert(any_side_fr.id());
        if let Some(initial_timeslot) = any_side_fr.time_bounds().lower_bound() {
            match self.chronology.1.entry(initial_timeslot) {
                Entry::Vacant(e) => {
                    let mut fresh_fragments = Fragments::new();
                    match fr {
                        OneSideLiquidity::Bid(fr) => fresh_fragments.bids.insert(fr),
                        OneSideLiquidity::Ask(fr) => fresh_fragments.asks.insert(fr),
                    };
                    e.insert(fresh_fragments);
                }
                Entry::Occupied(e) => {
                    match fr {
                        OneSideLiquidity::Bid(fr) => e.into_mut().bids.insert(fr),
                        OneSideLiquidity::Ask(fr) => e.into_mut().asks.insert(fr),
                    };
                }
            }
        }
    }
}

pub struct PooledLiquidity {
    pools: HashMap<SourceId, AnyPool>,
}

impl PooledLiquidity {
    fn best_pool(&mut self) -> Option<AnyPool> {
        // Best pool is determined by price and liquidity.
        todo!()
    }
}

/// Aggregates composable liquidity in three dimensions: price, volume, time.
pub struct TemporalLiquidityBook {
    fragmented_liquidity: FragmentedLiquidity<Slot, AnyFragment>,
    pooled_liquidity: PooledLiquidity,
}

impl TemporalLiquidityBook {
    fn match_one(
        &mut self,
        amount: TaggedAmount<Base>,
        price: Price,
        side: Side,
    ) -> Option<Either<AnyFragment, AnyPool>> {
        self.fragmented_liquidity
            .try_pop_best(!side, |fr| side.overlaps(price, fr.price()))
            .map(Either::Left)
            .or_else(|| {
                self.pooled_liquidity.best_pool().and_then(|best_pool| {
                    let pool_price = best_pool.price_hint();
                    if side.overlaps(price, pool_price) {
                        Some(Either::Right(best_pool))
                    } else {
                        None
                    }
                })
            })
    }
}

impl LiquidityBook<MarketEffect<Slot>> for TemporalLiquidityBook {
    fn apply_effect(&mut self, eff: MarketEffect<Slot>) {
        match eff {
            MarketEffect::ClocksAdvanced(new_time) => self.fragmented_liquidity.advance_clocks(new_time),
            MarketEffect::FragmentAdded(fr) => self.fragmented_liquidity.add_fragment(fr),
            MarketEffect::FragmentRemoved(id) => self.fragmented_liquidity.remove_fragment(id),
            MarketEffect::PoolUpdated(new_pool) => self.pooled_liquidity.insert(new_pool),
        }
    }
    fn pull_candidates(&self) -> Option<MatchCandidates> {
        //1. select one best fr from either side.
        //2. try to fill with one best fr from the opposite side.
        //3.a. if successful, test for imbalance from either side.
        //3.a.a. if there is imbalance, go to (2.).
        //3.a.b. else, return Some(match).
        //3.b. else, try to fill from best pool.
        //3.b.a if successful, return Some(match).
        //3.b.b else, return None.
        None
    }
}
