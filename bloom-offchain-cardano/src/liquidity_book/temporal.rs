use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::mem;

use cml_core::Slot;
use futures::future::Either;

use crate::liquidity_book::effect::Effect;
use crate::liquidity_book::fragment::Fragment;
use crate::liquidity_book::pool::Pool;
use crate::liquidity_book::recipe::{ExecutionRecipe, Fill, PartialFill, Swap, TerminalInstruction};
use crate::liquidity_book::side::{Side, SideMarker};
use crate::liquidity_book::types::{ExecutionCost, Price, SourceId};
use crate::liquidity_book::LiquidityBook;

pub struct ExecutionCap {
    soft: ExecutionCost,
    hard: ExecutionCost,
}

impl ExecutionCap {
    fn safe_threshold(&self) -> ExecutionCost {
        self.hard - self.soft
    }
}

pub struct TemporalLiquidityBook<FL, PL> {
    fragmented_liquidity: FL,
    pooled_liquidity: PL,
    execution_cap: ExecutionCap,
}

type Recipe = ExecutionRecipe<Fragment<Slot>, Pool>;

impl<FL, PL> TemporalLiquidityBook<FL, PL> {
    fn fill_once(&mut self, acc: Recipe, limit: ExecutionCost) -> Result<(Recipe, ExecutionCost), Recipe> {
        Err(acc)
    }
}

impl<FL, PL> LiquidityBook<Slot, Effect<Slot>> for TemporalLiquidityBook<FL, PL>
where
    FL: FragmentedLiquidity<Slot, Fragment<Slot>>,
    PL: PooledLiquidity<Pool>,
{
    fn apply(&mut self, effect: Effect<Slot>) {
        todo!()
    }
    fn attempt(&mut self) -> Option<ExecutionRecipe<Fragment<Slot>, Pool>> {
        if let Some(best_fr) = self.fragmented_liquidity.pick_either() {
            let mut acc = Recipe::new(best_fr);
            let mut execution_units_left = self.execution_cap.hard;
            loop {
                if let Some(rem) = &acc.remainder {
                    let price_fragments = self.fragmented_liquidity.best_price();
                    let price_pools = self.pooled_liquidity.best_price();
                    if price_fragments > price_pools
                        && execution_units_left > self.execution_cap.safe_threshold()
                    {
                        if let Some(opposite_fr) = self.fragmented_liquidity.try_pick(!rem.marker(), |fr| {
                            rem.map(|fr| fr.target.price).overlaps(fr.price)
                                && fr.cost_hint <= execution_units_left
                        }) {
                            execution_units_left -= opposite_fr.cost_hint;
                            match fill_from_fragment(*rem, opposite_fr) {
                                (term_fill_lt, Either::Left(term_fill_rt)) => {
                                    acc.push(TerminalInstruction::Fill(term_fill_lt));
                                    acc.terminate(TerminalInstruction::Fill(term_fill_rt));
                                    break;
                                }
                                (term_fill_lt, Either::Right(partial)) => {
                                    acc.push(TerminalInstruction::Fill(term_fill_lt));
                                    acc.set_remainder(partial);
                                }
                            }
                        }
                    } else {
                        if let Some(pool) = self.pooled_liquidity.try_pick(|pl| {
                            rem.map(|fr| fr.target.price)
                                .overlaps(pl.real_price(rem.marker(), rem.any().remainder))
                        }) {
                            let (term_fill, swap) = fill_from_pool(*rem, pool);
                            acc.push(TerminalInstruction::Swap(swap));
                            acc.terminate(TerminalInstruction::Fill(term_fill));
                            break;
                        }
                    }
                }
                break;
            }
            return Some(acc);
        }
        None
    }
}

fn fill_from_fragment<T>(
    target: Side<PartialFill<Fragment<T>>>,
    source: Fragment<T>,
) -> (
    Fill<Fragment<T>>,
    Either<Fill<Fragment<T>>, Side<PartialFill<Fragment<T>>>>,
) {
    todo!()
}

fn fill_from_pool<T>(
    target: Side<PartialFill<Fragment<T>>>,
    source: Pool,
) -> (Fill<Fragment<T>>, Swap<Pool>) {
    todo!()
}

trait FragmentedLiquidity<T, Fr> {
    fn best_price(&self) -> Price;
    fn pick_either(&mut self) -> Option<Side<Fr>>;
    fn try_pick<F>(&mut self, side: SideMarker, test: F) -> Option<Fr>
    where
        F: FnOnce(&Fr) -> bool;
}

trait PooledLiquidity<Pl> {
    fn best_price(&self) -> Price;
    fn try_pick<F>(&mut self, test: F) -> Option<Pl>
    where
        F: FnOnce(&Pl) -> bool;
}

#[derive(Debug, Clone)]
pub struct PooledLiquidityMem<Pl> {
    pools: HashMap<SourceId, Pl>,
}

#[derive(Debug, Clone)]
pub struct FragmentedLiquidityMem<T, Fr> {
    /// Liquidity fragments spread across time axis.
    /// First element in the tuple encodes the time point which is now.
    chronology: (Fragments<Fr>, BTreeMap<T, Fragments<Fr>>),
    known_fragments: HashSet<SourceId>,
    removed_fragments: HashSet<SourceId>,
}

impl<T> FragmentedLiquidityMem<T, Fragment<T>>
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
