use std::collections::{btree_map, BTreeMap, BTreeSet, HashMap};
use std::collections::hash_map::Entry;
use std::mem;

use spectrum_offchain::data::Has;

use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use crate::execution_engine::liquidity_book::side::{Side, SideMarker};
use crate::execution_engine::liquidity_book::types::Price;
use crate::execution_engine::SourceId;

/// State with no uncommitted changes.
pub struct SettledState<Fr, Pl> {
    fragments: Chronology<Fr>,
    pools: Pools<Pl>,
}

impl<Fr, Pl> SettledState<Fr, Pl> {
    fn new(time_now: u64) -> Self {
        Self {
            fragments: Chronology::new(time_now),
            pools: Pools::new(),
        }
    }
}

impl<Fr, Pl> SettledState<Fr, Pl> where Fr: Clone, Pl: Clone {
    fn into_preview(self) -> PreviewState<Fr, Pl> {
        PreviewState {
            active_fragments_preview: None,
            fragments_intact: self.fragments,
            inactive_fragments_changeset: vec![],
            pools_preview: self.pools.clone(),
            pools_intact: self.pools,
        }
    }
}

/// State with areas of uncommitted changes.
pub struct PreviewState<Fr, Pl> {
    /// Fragments before changes.
    fragments_intact: Chronology<Fr>,
    /// Active fragments with changes pre-applied.
    active_fragments_preview: Option<Fragments<Fr>>,
    /// Set of new inactive fragments.
    inactive_fragments_changeset: Vec<(u64, Fr)>,
    /// Pools before changes.
    pools_intact: Pools<Pl>,
    /// Active pools with changes pre-applied.
    pools_preview: Pools<Pl>,
}

impl<Fr, Pl> PreviewState<Fr, Pl> {
    pub fn new(time_now: u64) -> Self {
        Self {
            fragments_intact: Chronology::new(time_now),
            active_fragments_preview: None,
            inactive_fragments_changeset: vec![],
            pools_intact: Pools::new(),
            pools_preview: Pools::new(),
        }
    }
}

pub enum TLBState<Fr, Pl> {
    Settled(SettledState<Fr, Pl>),
    Preview(PreviewState<Fr, Pl>),
}

impl<Fr, Pl> TLBState<Fr, Pl> where Fr: Fragment + Ord + Copy, Pl: QualityMetric + Has<SourceId> + Copy {
    pub fn fragments(&self) -> &Fragments<Fr> {
        match self {
            TLBState::Settled(st) => &st.fragments.active,
            TLBState::Preview(st) => match &st.active_fragments_preview {
                None => &st.fragments_intact.active,
                Some(active) => active,
            },
        }
    }

    pub fn fragments_mut(&mut self) -> &mut Fragments<Fr> {
        match self {
            TLBState::Settled(st) => &mut st.fragments.active,
            TLBState::Preview(st) => match &mut st.active_fragments_preview {
                None => &mut st.fragments_intact.active,
                Some(active) => active,
            },
        }
    }

    pub fn pools(&self) -> &Pools<Pl> {
        match self {
            TLBState::Settled(st) => &st.pools,
            TLBState::Preview(st) => &st.pools_preview,
        }
    }

    pub fn pools_mut(&mut self) -> &mut Pools<Pl> {
        match self {
            TLBState::Settled(st) => &mut st.pools,
            TLBState::Preview(st) => &mut st.pools_preview,
        }
    }

    pub fn pre_add_fragment(&mut self, fr: Fr) {
        let time = self.current_time();
        match (self, fr.time_bounds().lower_bound()) {
            // We have to transit to preview state.
            (this@TLBState::Settled(_), lower_bound) => {
                let mut preview_st = PreviewState::new(time);
                match this {
                    TLBState::Settled(st) => {
                        mem::swap(&mut preview_st.fragments_intact, &mut st.fragments);
                        mem::swap(&mut preview_st.pools_intact, &mut st.pools);
                        match lower_bound {
                            // If `fr` is inactive we avoid cloning active frontier.
                            Some(lower_bound) if lower_bound > time => {
                                preview_st.inactive_fragments_changeset.push((lower_bound, fr));
                            },
                            // Otherwise we have to create a copy of active frontier and add new `fr` to it.
                            _ => {
                                let mut active_fragments = mem::replace(&mut st.fragments.active, Fragments::new());
                                active_fragments.insert(fr);
                                preview_st.active_fragments_preview.replace(active_fragments);
                            },
                        }
                    },
                    TLBState::Preview(_) => unreachable!(),
                }
                mem::swap(this, &mut TLBState::Preview(preview_st));
            }
            (TLBState::Preview(ref mut preview_st), lower_bound) => {
                match lower_bound {
                    Some(lb) if lb > time => preview_st.inactive_fragments_changeset.push((lb, fr)),
                    _ => {
                        match &mut preview_st.active_fragments_preview {
                            Some(active_preview) => active_preview.insert(fr),
                            this_active_preview => {
                                let mut active_preview = preview_st.fragments_intact.active.clone();
                                active_preview.insert(fr);
                                this_active_preview.replace(active_preview);
                            }
                        }
                    },
                }
            }
        }
    }

    pub fn pre_add_pool(&mut self, pool: Pl) {
        match self {
            this@TLBState::Settled(_) => {
                let empty_state = SettledState::new(0);
                let settled_st = mem::replace(this, TLBState::Settled(empty_state));
                let mut preview_st = match settled_st {
                    TLBState::Settled(settled_st) => settled_st.into_preview(),
                    TLBState::Preview(_) => unreachable!()
                };
                preview_st.pools_preview.update_pool(pool);
                mem::swap(this, &mut TLBState::Preview(preview_st));
            }
            TLBState::Preview(ref mut state) => {
                state.pools_preview.update_pool(pool);
            }
        }
    }

    pub fn current_time(&self) -> u64 {
        match self {
            TLBState::Settled(st) => st.fragments.time_now,
            TLBState::Preview(st) => st.fragments_intact.time_now,
        }
    }
}

/// Liquidity fragments spread across time axis.
#[derive(Debug, Clone)]
struct Chronology<Fr> {
    time_now: u64,
    active: Fragments<Fr>,
    inactive: BTreeMap<u64, Fragments<Fr>>,
    index: HashMap<SourceId, Fr>,
}

impl<Fr> Chronology<Fr> {
    pub fn new(time_now: u64) -> Self {
        Self {
            time_now,
            active: Fragments::new(),
            inactive: BTreeMap::new(),
            index: HashMap::new(),
        }
    }
}

impl<Fr> Chronology<Fr>
where
    Fr: Fragment + OrderState + Ord + Copy,
{
    fn advance_clocks(&mut self, new_time: u64) {
        let new_slot = self
            .inactive
            .remove(&new_time)
            .unwrap_or_else(|| Fragments::new());
        let Fragments { asks, bids } = mem::replace(&mut self.active, new_slot);
        for fr in asks {
            if let StateTrans::Active(next_fr) = fr.with_updated_time(new_time) {
                self.active.asks.insert(next_fr);
            }
        }
        for fr in bids {
            if let StateTrans::Active(next_fr) = fr.with_updated_time(new_time) {
                self.active.bids.insert(next_fr);
            }
        }
        self.time_now = new_time;
    }

    fn remove_fragments(&mut self, source: SourceId) {
        if let Some(fr) = self.index.remove(&source) {
            if let Some(initial_timeslot) = fr.time_bounds().lower_bound() {
                if initial_timeslot <= self.time_now {
                    match fr.side() {
                        SideMarker::Bid => self.active.bids.remove(&fr),
                        SideMarker::Ask => self.active.asks.remove(&fr),
                    };
                } else {
                    match self.inactive.entry(initial_timeslot) {
                        btree_map::Entry::Occupied(e) => {
                            match fr.side() {
                                SideMarker::Bid => e.into_mut().bids.remove(&fr),
                                SideMarker::Ask => e.into_mut().asks.remove(&fr),
                            };
                        }
                        btree_map::Entry::Vacant(_) => {}
                    }
                }
            }
        }
    }

    fn add_fragment(&mut self, fr: Fr) {
        self.index.insert(fr.source(), fr);
        if let Some(initial_timeslot) = fr.time_bounds().lower_bound() {
            if initial_timeslot <= self.time_now {
                self.active.insert(fr);
            } else {
                match self.inactive.entry(initial_timeslot) {
                    btree_map::Entry::Vacant(e) => {
                        let mut fresh_fragments = Fragments::new();
                        fresh_fragments.insert(fr);
                        e.insert(fresh_fragments);
                    }
                    btree_map::Entry::Occupied(e) => {
                        e.into_mut().insert(fr);
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

impl<Fr> Fragments<Fr>
where
    Fr: Fragment + Ord,
{
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
        F: FnOnce(&Fr) -> bool,
    {
        let side = match side {
            SideMarker::Bid => &mut self.bids,
            SideMarker::Ask => &mut self.asks,
        };
        side.pop_first()
            .and_then(|best_bid| if test(&best_bid) { Some(best_bid) } else { None })
    }

    pub fn insert(&mut self, fr: Fr) {
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

impl<Pl> Pools<Pl> {
    pub fn new() -> Self {
        Self {
            pools: HashMap::new(),
            quality_index: BTreeMap::new(),
        }
    }
}

impl<Pl> Pools<Pl> where Pl: QualityMetric + Has<SourceId> + Copy {
    pub fn best_price(&self) -> Option<Price> {
        self.quality_index
            .first_key_value()
            .map(|(PoolQuality(p, _), _)| *p)
    }

    pub fn try_pick<F>(&mut self, test: F) -> Option<Pl>
    where
        F: Fn(&Pl) -> bool,
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

    pub fn update_pool(&mut self, pool: Pl) {
        let source = pool.get::<SourceId>();
        if let Some(old_pool) = self.pools.insert(source, pool) {
            self.quality_index.remove(&old_pool.quality());
            self.quality_index.insert(pool.quality(), source);
        }
    }
}

pub trait QualityMetric {
    fn quality(&self) -> PoolQuality;
}

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PoolQuality(/*price hint*/ Price, /*liquidity*/ u64);
