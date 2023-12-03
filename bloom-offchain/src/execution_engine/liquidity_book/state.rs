use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::collections::hash_map::Entry;

use spectrum_offchain::data::Has;

use crate::execution_engine::liquidity_book::fragment::Fragment;
use crate::execution_engine::liquidity_book::side::{Side, SideMarker};
use crate::execution_engine::liquidity_book::types::Price;
use crate::execution_engine::SourceId;

/// State with no uncommitted changes.
pub struct SettledState<Fr, Pl> {
    fragments: Chronology<Fr>,
    pools: Pools<Pl>,
}

/// State with areas of uncommitted changes.
pub struct UnsettledState<Fr, Pl> {
    /// Fragments before changes.
    prev_fragments: Chronology<Fr>,
    /// Active fragments with changes applied.
    active_fragments_applied: Fragments<Fr>,
    /// Set of new inactive fragments.
    inactive_fragments_changeset: Vec<(u64, Fr)>,
    /// Pools before changes.
    prev_pools: Pools<Pl>,
    /// Active pools with changes applied.
    active_pools: Pools<Pl>,
}

pub enum TLBState<Fr, Pl> {
    Settled(SettledState<Fr, Pl>),
    Unsettled(UnsettledState<Fr, Pl>),
}

impl<Fr, Pl> TLBState<Fr, Pl> {
    pub fn fragments(&self) -> &Fragments<Fr> {
        match self {
            TLBState::Settled(st) => &st.fragments.active,
            TLBState::Unsettled(st) => &st.active_fragments_applied,
        }
    }
    pub fn fragments_mut(&mut self) -> &mut Fragments<Fr> {
        match self {
            TLBState::Settled(st) => &mut st.fragments.active,
            TLBState::Unsettled(st) => &mut st.active_fragments_applied,
        }
    }
    pub fn pools(&self) -> &Pools<Pl> {
        match self {
            TLBState::Settled(st) => &st.pools,
            TLBState::Unsettled(st) => &st.active_pools,
        }
    }
    pub fn pools_mut(&mut self) -> &mut Pools<Pl> {
        match self {
            TLBState::Settled(st) => &mut st.pools,
            TLBState::Unsettled(st) => &mut st.active_pools,
        }
    }
}

/// Liquidity fragments spread across time axis.
#[derive(Debug, Clone)]
struct Chronology<Fr> {
    time_now: u64,
    active: Fragments<Fr>,
    inactive: BTreeMap<u64, Fragments<Fr>>,
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

impl<Fr> Fragments<Fr> where Fr: Fragment + Ord {
    pub fn best_price(&self, side: SideMarker) -> Option<Side<Price>> {
        let side_store = match side {
            SideMarker::Bid => &self.bids,
            SideMarker::Ask => &self.asks,
        };
        side_store.first().map(|fr| side.wrap(fr.price()))
    }
    
    pub fn pick_either(&mut self) -> Option<Fr> {
        let best_bid = self.bids.pop_first();
        let best_ask = self.asks.pop_first();
        match (best_bid, best_ask) {
            (Some(bid), Some(ask)) if bid.weight() >= ask.weight() => Some(bid),
            (Some(_), Some(ask)) => Some(ask),
            (Some(any), None) | (None, Some(any)) => Some(any),
            _ => None,
        }
    }
    
    pub fn try_pick<F>(&mut self, side: SideMarker, test: F) -> Option<Fr>
        where
            F: FnOnce(&Fr) -> bool {
        let side = match side {
            SideMarker::Bid => &mut self.bids,
            SideMarker::Ask => &mut self.asks,
        };
        side.pop_first()
            .and_then(|best_bid| if test(&best_bid) { Some(best_bid) } else { None })
    }
    
    pub fn return_fr(&mut self, fr: Fr) {
        match fr.side() {
            SideMarker::Bid => self.bids.insert(fr),
            SideMarker::Ask => self.bids.insert(fr),
        };
    }
}

#[derive(Debug, Clone)]
pub struct Pools<Pl> {
    pools: HashMap<SourceId, Pl>,
    quality_index: BTreeMap<PoolQuality, SourceId>,
}

impl<Pl: Has<SourceId>> Pools<Pl> {
    pub fn best_price(&self) -> Option<Price> {
        self.quality_index
            .first_key_value()
            .map(|(PoolQuality(p, _), _)| *p)
    }
    
    pub fn try_pick<F>(&mut self, test: F) -> Option<Pl>
        where
            F: Fn(&Pl) -> bool
    {
        for id in self.quality_index.values() {
            match self.pools.entry(*id) {
                Entry::Occupied(pl) if test(pl.get()) => return Some(pl.remove()),
                _ => {}
            }
        }
        None
    }
    
    pub fn return_pool(&mut self, pool: Pl) {
        self.pools.insert(pool.get::<SourceId>(), pool);
    }
}

pub trait QualityMetric {
    fn quality(&self) -> PoolQuality;
}

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PoolQuality(/*price hint*/ Price, /*liquidity*/ u64);

