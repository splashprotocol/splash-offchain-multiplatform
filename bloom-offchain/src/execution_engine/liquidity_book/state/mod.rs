use crate::execution_engine::liquidity_book::core::Next;
use crate::execution_engine::liquidity_book::market_maker::{AvailableLiquidity, MarketMaker, PoolQuality};
use crate::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use crate::execution_engine::liquidity_book::side::{OnSide, Side};
use crate::execution_engine::liquidity_book::stashing_option::StashingOption;
use crate::execution_engine::liquidity_book::state::price_range::AllowedPriceRange;
use crate::execution_engine::liquidity_book::types::AbsolutePrice;
use crate::execution_engine::liquidity_book::weight::Weighted;
use either::{Either, Left, Right};
use log::trace;
use serde::{Deserialize, Serialize};
use spectrum_offchain::display::display_vec;
use spectrum_offchain::domain::Stable;
use std::collections::{btree_map, BTreeMap, BTreeSet, HashMap};
use std::fmt::{Debug, Display, Formatter};
use std::mem;
use std::ops::Add;

mod price_range;
pub mod queries;

#[derive(Clone, Eq, PartialEq)]
/// State with no uncommitted changes.
pub struct IdleState<T, M: Stable> {
    takers: Chronology<T>,
    makers: MarketMakers<M>,
}

impl<T, M: Stable> IdleState<T, M> {
    fn new(time_now: u64) -> Self {
        Self {
            takers: Chronology::new(time_now),
            makers: MarketMakers::new(),
        }
    }
}

impl<T, M> IdleState<T, M>
where
    T: MarketTaker + TakerBehaviour + Ord + Copy + Display,
    M: MarketMaker + Stable + Copy + Display + Debug,
{
    pub fn advance_clocks(&mut self, new_time: u64) {
        self.takers.advance_clocks(new_time)
    }

    pub fn add_fragment(&mut self, fr: T) {
        trace!("Adding {} to active frontier", fr);
        self.takers.add_fragment(fr);
    }

    pub fn remove_fragment(&mut self, fr: T) {
        trace!("Removing {} from active frontier", fr);
        self.takers.remove_fragment(fr);
    }

    pub fn update_pool(&mut self, maker: M) {
        trace!("Updating {} in active frontier", maker);
        self.makers.update_pool(maker);
    }

    pub fn remove_pool(&mut self, maker: M) {
        trace!("Removing {} from active frontier", maker);
        self.makers.remove_pool(maker);
    }
}

/// Changed state that reflects only consumption of fragments and full preview of makers.
/// We use this one when no preview takers/makers are generated to avoid
/// overhead of copying active frontier projection.
#[derive(Clone, Eq, PartialEq)]
pub struct PartialPreviewState<T, M: Stable> {
    takers_preview: Chronology<T>,
    consumed_active_takers: Vec<T>,
    stashed_active_takers: Vec<T>,
    makers_intact: MarketMakers<M>,
    makers_preview: MarketMakers<M>,
}

impl<T, M: Stable> PartialPreviewState<T, M> {
    pub fn new(time_now: u64) -> Self {
        Self {
            takers_preview: Chronology::new(time_now),
            consumed_active_takers: vec![],
            stashed_active_takers: vec![],
            makers_intact: MarketMakers::new(),
            makers_preview: MarketMakers::new(),
        }
    }
}

impl<T, M: Stable> PartialPreviewState<T, M>
where
    T: MarketTaker + Ord,
    M: Copy,
{
    fn commit(&mut self) -> IdleState<T, M> {
        trace!(target: "state", "PartialPreviewState::commit");
        let mut fresh_settled_st = IdleState::new(0);
        mem::swap(&mut fresh_settled_st.takers, &mut self.takers_preview);
        mem::swap(&mut fresh_settled_st.makers, &mut self.makers_preview);
        fresh_settled_st
    }

    fn rollback(
        &mut self,
        stashing_opt: StashingOption<T>,
    ) -> Either<IdleState<T, M>, PartialPreviewState<T, M>> {
        trace!(target: "state", "PartialPreviewState::rollback");
        // Return consumed fragments to reconstruct initial state.
        while let Some(fr) = self.consumed_active_takers.pop() {
            self.takers_preview.active.insert(fr);
        }
        match stashing_opt {
            StashingOption::Stash(mut to_stash) => {
                for fr in &to_stash {
                    self.takers_preview.active.remove(fr);
                }
                self.stashed_active_takers.append(&mut to_stash);
            }
            StashingOption::Unstash => {
                self.unstash();
            }
        }
        if self.stashed_active_takers.is_empty() {
            trace!("PartialPreviewState => IdleState");
            let mut fresh_idle_st = IdleState::new(0);
            // Move reconstructed initial fragments into idle state.
            mem::swap(&mut self.takers_preview, &mut fresh_idle_st.takers);
            mem::swap(&mut self.makers_intact, &mut fresh_idle_st.makers);
            Left(fresh_idle_st)
        } else {
            trace!("PartialPreviewState => PartialPreviewState[With Stash]");
            let mut fresh_preview_st = PartialPreviewState::new(self.takers_preview.time_now);
            // Move reconstructed initial fragments into idle state.
            mem::swap(&mut self.takers_preview, &mut fresh_preview_st.takers_preview);
            mem::swap(
                &mut self.stashed_active_takers,
                &mut fresh_preview_st.stashed_active_takers,
            );
            mem::swap(&mut self.makers_intact, &mut fresh_preview_st.makers_intact);
            fresh_preview_st.makers_preview = fresh_preview_st.makers_intact.clone();
            Right(fresh_preview_st)
        }
    }

    fn unstash(&mut self) {
        let stashed_fragments = mem::take(&mut self.stashed_active_takers);
        for fr in stashed_fragments {
            self.takers_preview.active.insert(fr);
        }
    }
}

/// State with areas of uncommitted changes.
/// This state offers consistent projections of active frontier for both
/// consumption and production of new fragments/pools.
/// Comes with overhead of cloning active frontier/pools upon construction.
#[derive(Clone, Eq, PartialEq)]
pub struct PreviewState<T, M: Stable> {
    /// Fragments before changes.
    takers_intact: Chronology<T>,
    /// Active fragments with changes pre-applied.
    active_takers_preview: MarketTakers<T>,
    stashed_active_takers: Vec<T>,
    /// Set of new inactive fragments.
    inactive_takers_changeset: Vec<(u64, T)>,
    /// Pools before changes.
    makers_intact: MarketMakers<M>,
    /// Active pools with changes pre-applied.
    makers_preview: MarketMakers<M>,
}

impl<Fr, Pl: Stable> PreviewState<Fr, Pl> {
    fn new(time_now: u64) -> Self {
        Self {
            takers_intact: Chronology::new(time_now),
            active_takers_preview: MarketTakers::new(),
            inactive_takers_changeset: vec![],
            stashed_active_takers: vec![],
            makers_intact: MarketMakers::new(),
            makers_preview: MarketMakers::new(),
        }
    }
}

impl<Fr, Pl> PreviewState<Fr, Pl>
where
    Fr: MarketTaker + Ord,
    Pl: Stable + Copy,
{
    fn commit(&mut self) -> IdleState<Fr, Pl> {
        trace!(target: "state", "PreviewState::commit");
        // Commit active fragments preview if available.
        mem::swap(&mut self.takers_intact.active, &mut self.active_takers_preview);
        // Commit inactive fragments.
        while let Some((time, t)) = self.inactive_takers_changeset.pop() {
            match self.takers_intact.inactive.entry(time) {
                btree_map::Entry::Vacant(entry) => {
                    let mut takers = MarketTakers::new();
                    takers.insert(t);
                    entry.insert(takers);
                }
                btree_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().insert(t);
                }
            }
        }
        let mut fresh_settled_st = IdleState::new(self.takers_intact.time_now);
        self.unstash();
        mem::swap(&mut fresh_settled_st.takers, &mut self.takers_intact);
        // Commit pools preview if available.
        mem::swap(&mut fresh_settled_st.makers, &mut self.makers_preview);
        fresh_settled_st
    }

    fn unstash(&mut self) {
        let stashed_fragments = mem::take(&mut self.stashed_active_takers);
        for fr in stashed_fragments {
            self.takers_intact.active.insert(fr);
        }
    }

    fn rollback(
        &mut self,
        stashing_opt: StashingOption<Fr>,
    ) -> Either<IdleState<Fr, Pl>, PartialPreviewState<Fr, Pl>> {
        match stashing_opt {
            StashingOption::Stash(mut to_stash) => {
                for fr in &to_stash {
                    self.takers_intact.active.remove(fr);
                }
                self.stashed_active_takers.append(&mut to_stash);
            }
            StashingOption::Unstash => {
                self.unstash();
            }
        }
        if self.stashed_active_takers.is_empty() {
            trace!("PreviewState => IdleState");
            let mut fresh_settled_st = IdleState::new(self.takers_intact.time_now);
            mem::swap(&mut fresh_settled_st.takers, &mut self.takers_intact);
            mem::swap(&mut fresh_settled_st.makers, &mut self.makers_intact);
            Left(fresh_settled_st)
        } else {
            trace!("PreviewState => PartialPreviewState[With Stash]");
            let mut fresh_preview_st = PartialPreviewState::new(self.takers_intact.time_now);
            mem::swap(&mut fresh_preview_st.takers_preview, &mut self.takers_intact);
            mem::swap(
                &mut fresh_preview_st.stashed_active_takers,
                &mut self.stashed_active_takers,
            );
            mem::swap(&mut fresh_preview_st.makers_intact, &mut self.makers_intact);
            fresh_preview_st.makers_preview = fresh_preview_st.makers_intact.clone();
            Right(fresh_preview_st)
        }
    }
}

/// The idea of TLB state automata is to minimize overhead of maintaining preview of modified state.
#[derive(Clone, Eq, PartialEq)]
pub enum TLBState<T, M: Stable> {
    /// State with no uncommitted changes.
    ///
    ///              Idle
    ///              |  \
    /// PartialPreview   Preview
    Idle(IdleState<T, M>),
    /// Modified state that reflects only consumption of fragments and full preview of pools.
    ///
    ///          PartialPreview
    ///              |  \
    ///           Idle   Preview
    PartialPreview(PartialPreviewState<T, M>),
    /// State with areas of uncommitted changes: consumption and production of fragments/pools.
    ///
    ///             Preview
    ///                |
    ///              Idle
    Preview(PreviewState<T, M>),
}

impl<T: Stable, M: Stable> Display for TLBState<T, M> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            match self {
                TLBState::Idle(st) => format!(
                    "TLBState::Idle(active_asks: {}, active_bids: {}, active_mms: {})",
                    display_vec(&st.takers.active.asks.iter().map(|tk| tk.stable_id()).collect()),
                    display_vec(&st.takers.active.bids.iter().map(|tk| tk.stable_id()).collect()),
                    display_vec(&st.makers.values.keys().collect())
                ),
                TLBState::PartialPreview(st) => format!(
                    "TLBState::PartialPreview(active_asks: {}, active_bids: {}, active_mms: {})",
                    display_vec(
                        &st.takers_preview
                            .active
                            .asks
                            .iter()
                            .map(|tk| tk.stable_id())
                            .collect()
                    ),
                    display_vec(
                        &st.takers_preview
                            .active
                            .bids
                            .iter()
                            .map(|tk| tk.stable_id())
                            .collect()
                    ),
                    display_vec(&st.makers_preview.values.keys().collect())
                ),
                TLBState::Preview(st) => format!(
                    "TLBState::Preview(active_asks: {}, active_bids: {}, active_mms: {})",
                    display_vec(
                        &st.active_takers_preview
                            .asks
                            .iter()
                            .map(|tk| tk.stable_id())
                            .collect()
                    ),
                    display_vec(
                        &st.active_takers_preview
                            .bids
                            .iter()
                            .map(|tk| tk.stable_id())
                            .collect()
                    ),
                    display_vec(&st.makers_preview.values.keys().collect())
                ),
            }
            .as_str(),
        )
    }
}

impl<T, M: Stable> TLBState<T, M> {
    pub fn new(time: u64) -> Self {
        Self::Idle(IdleState::new(time))
    }
    pub fn size(&self) -> LiquidityBookSize {
        match self {
            TLBState::Idle(idle) => LiquidityBookSize {
                num_active_takers: idle.takers.active.asks.len() + idle.takers.active.bids.len(),
                num_active_makers: idle.makers.values.len(),
                num_idle_takers: idle.takers.inactive.len(),
                num_idle_makers: 0,
            },
            TLBState::PartialPreview(pp) => LiquidityBookSize {
                num_active_takers: pp.takers_preview.active.asks.len() + pp.takers_preview.active.bids.len(),
                num_active_makers: pp.makers_preview.values.len(),
                num_idle_takers: pp.takers_preview.inactive.len(),
                num_idle_makers: 0,
            },
            TLBState::Preview(p) => LiquidityBookSize {
                num_active_takers: p.active_takers_preview.asks.len() + p.active_takers_preview.bids.len(),
                num_active_makers: p.makers_preview.values.len(),
                num_idle_takers: p.takers_intact.inactive.len() + p.inactive_takers_changeset.len(),
                num_idle_makers: 0,
            },
        }
    }
}

impl<T, M: Stable> TLBState<T, M>
where
    T: MarketTaker + Ord + Copy,
{
    fn active_fragments(&self) -> &MarketTakers<T> {
        match self {
            TLBState::Idle(st) => &st.takers.active,
            TLBState::PartialPreview(st) => &st.takers_preview.active,
            TLBState::Preview(st) => &st.active_takers_preview,
        }
    }
}

impl<T, M> TLBState<T, M>
where
    T: MarketTaker + Ord + Copy,
    M: Stable + Copy,
{
    pub fn commit(&mut self) {
        match self {
            TLBState::PartialPreview(st) => {
                trace!(target: "tlb", "TLBState::PartialPreview: commit");
                let new_st = st.commit();
                mem::swap(self, &mut TLBState::Idle(new_st));
            }
            TLBState::Preview(st) => {
                trace!(target: "tlb", "TLBState::Preview: commit");
                let new_st = st.commit();
                mem::swap(self, &mut TLBState::Idle(new_st));
            }
            TLBState::Idle(_) => {}
        }
    }

    pub fn rollback(&mut self, stashing_opt: StashingOption<T>) {
        match self {
            TLBState::PartialPreview(st) => {
                trace!(target: "tlb", "TLBState::PartialPreview: rollback");
                let mut new_st = st
                    .rollback(stashing_opt)
                    .either(TLBState::Idle, TLBState::PartialPreview);
                mem::swap(self, &mut new_st);
            }
            TLBState::Preview(st) => {
                trace!(target: "tlb", "TLBState::Preview: rollback");
                let mut new_st = st
                    .rollback(stashing_opt)
                    .either(TLBState::Idle, TLBState::PartialPreview);
                mem::swap(self, &mut new_st);
            }
            TLBState::Idle(_) => {}
        }
    }

    fn move_into_partial_preview(&mut self, target: &mut PartialPreviewState<T, M>) {
        match self {
            // Transit into PartialPreview if state is untouched yet
            TLBState::Idle(st) => {
                trace!("move Idle => PartialPreview");
                // Move untouched takers/makers into fresh state.
                mem::swap(&mut target.takers_preview, &mut st.takers);
                mem::swap(&mut target.makers_intact, &mut st.makers);
                // Initialize pools preview with a copy of untouched makers.
                target.makers_preview = target.makers_intact.clone();
            }
            TLBState::PartialPreview(_) => panic!("Attempt to move PartialPreview => PartialPreview"),
            TLBState::Preview(_) => panic!("Attempt to move Preview => PartialPreview"),
        }
    }

    fn move_into_preview(&mut self, target: &mut PreviewState<T, M>) {
        match self {
            TLBState::Idle(st) => {
                trace!("move Idle => Preview");
                // Move untouched fragments/pools into preview state.
                mem::swap(&mut target.takers_intact, &mut st.takers);
                mem::swap(&mut target.makers_intact, &mut st.makers);
                // Move active fragments/pools to use as a preview.
                let mut active_fragments = target.takers_intact.active.clone();
                mem::swap(&mut target.active_takers_preview, &mut active_fragments);
                target.makers_preview = target.makers_intact.clone();
            }
            TLBState::PartialPreview(st) => {
                trace!("move PartialPreview => Preview");
                // Copy active fragments/pools to use as a preview.
                target.active_takers_preview = st.takers_preview.active.clone();
                mem::swap(&mut target.makers_preview, &mut st.makers_preview);
                mem::swap(&mut target.stashed_active_takers, &mut st.stashed_active_takers);
                // Return consumed takers to reconstruct initial state.
                while let Some(fr) = st.consumed_active_takers.pop() {
                    st.takers_preview.active.insert(fr);
                }
                // Move untouched state into preview.
                mem::swap(&mut target.takers_intact, &mut st.takers_preview);
                mem::swap(&mut target.makers_intact, &mut st.makers_intact);
            }
            TLBState::Preview(_) => panic!("Attempt to move Preview => Preview"),
        }
    }
}

impl<T, M, U> TLBState<T, M>
where
    T: MarketTaker<U = U> + Ord + Copy + Display,
    M: MarketMaker + Stable + Copy,
    U: PartialOrd,
{
    pub fn show_state(&self) -> String
    where
        M::StableId: Display,
        M: Display,
        T: Display,
    {
        let pools = self.pools().show_state();
        let fragments = self.active_fragments().show_state();
        format!("Fragments(active): {}, Pools: {}", fragments, pools)
    }

    pub fn allowed_price_range(&self) -> AllowedPriceRange {
        match self {
            TLBState::Idle(_) => AllowedPriceRange::default(),
            TLBState::PartialPreview(PartialPreviewState {
                stashed_active_takers,
                ..
            })
            | TLBState::Preview(PreviewState {
                stashed_active_takers,
                ..
            }) => AllowedPriceRange {
                max_ask_price: stashed_active_takers
                    .iter()
                    .filter_map(|tk| {
                        if tk.side() == Side::Ask {
                            Some(tk.price())
                        } else {
                            None
                        }
                    })
                    .min(),
                min_bid_price: stashed_active_takers
                    .iter()
                    .filter_map(|tk| {
                        if tk.side() == Side::Bid {
                            Some(tk.price())
                        } else {
                            None
                        }
                    })
                    .max(),
            },
        }
    }

    pub fn best_taker_price(&self, side: Side) -> Option<OnSide<AbsolutePrice>> {
        let active_fragments = self.active_fragments();
        let side_store = match side {
            Side::Bid => &active_fragments.bids,
            Side::Ask => &active_fragments.asks,
        };
        side_store.first().map(|fr| side.wrap(fr.price()))
    }

    /// Pick best fragment from either side
    pub fn pick_best_fr_either(&mut self, index_price: Option<AbsolutePrice>) -> Option<T> {
        trace!(target: "state", "pick_best_fr_either");
        self.pick_active_taker(|fragments| pick_best_fr_either(fragments, index_price))
    }

    /// Pick best fragment from the specified side if it matches the specified condition.
    pub fn try_pick_taker<F>(&mut self, side: Side, test: F) -> Option<T>
    where
        F: FnOnce(&T) -> bool,
    {
        trace!(target: "state", "try_pick_fr");
        self.pick_active_taker(|af| try_pick_fr(af, side, test))
    }

    /// Add preview fragment [T].
    pub fn pre_add_taker(&mut self, taker: T) {
        trace!("pre_add_taker");
        let time = self.current_time();
        match (self, taker.time_bounds().lower_bound()) {
            // We have to transit to preview state.
            (this @ TLBState::Idle(_) | this @ TLBState::PartialPreview(_), lower_bound) => {
                let mut preview_st = PreviewState::new(time);
                this.move_into_preview(&mut preview_st);
                // Add taker into preview.
                match lower_bound {
                    Some(lower_bound) if lower_bound > time => {
                        preview_st.inactive_takers_changeset.push((lower_bound, taker));
                    }
                    _ => preview_st.active_takers_preview.insert(taker),
                }
                mem::swap(this, &mut TLBState::Preview(preview_st));
            }
            (TLBState::Preview(ref mut preview_st), lower_bound) => match lower_bound {
                Some(lb) if lb > time => preview_st.inactive_takers_changeset.push((lb, taker)),
                _ => preview_st.active_takers_preview.insert(taker),
            },
        }
    }

    /// Add preview pool [M].
    pub fn pre_add_maker(&mut self, pool: M) {
        match self {
            this @ TLBState::Idle(_) | this @ TLBState::PartialPreview(_) => {
                let mut preview_st = PreviewState::new(0);
                this.move_into_preview(&mut preview_st);
                // Add pool into preview.
                preview_st.makers_preview.update_pool(pool);
                mem::swap(this, &mut TLBState::Preview(preview_st));
            }
            TLBState::Preview(ref mut state) => state.makers_preview.update_pool(pool),
        }
    }

    /// Pick active fragment ensuring TLB is in proper state.
    pub fn pick_active_taker<F>(&mut self, f: F) -> Option<T>
    where
        F: FnOnce(&mut MarketTakers<T>) -> Option<T>,
    {
        let mut needs_transition = false;
        let res = match self {
            // Transit into PartialPreview if state is untouched yet
            TLBState::Idle(idle_st) => {
                let active_fragments = &mut idle_st.takers.active;
                if let Some(choice) = f(active_fragments) {
                    needs_transition = true;
                    Some(choice)
                } else {
                    None
                }
            }
            TLBState::PartialPreview(busy_st) => {
                let active_fragments = &mut busy_st.takers_preview.active;
                if let Some(choice) = f(active_fragments) {
                    busy_st.consumed_active_takers.push(choice);
                    Some(choice)
                } else {
                    None
                }
            }
            TLBState::Preview(preview_st) => {
                let active_fragments = &mut preview_st.active_takers_preview;
                f(active_fragments)
            }
        };

        if needs_transition {
            let mut busy_st = PartialPreviewState::new(0);
            self.move_into_partial_preview(&mut busy_st);
            busy_st.consumed_active_takers.push(res.unwrap());
            mem::swap(self, &mut TLBState::PartialPreview(busy_st));
        }

        res
    }

    fn current_time(&self) -> u64 {
        match self {
            TLBState::Idle(st) => st.takers.time_now,
            TLBState::PartialPreview(st) => st.takers_preview.time_now,
            TLBState::Preview(st) => st.takers_intact.time_now,
        }
    }
}

impl<T, M> TLBState<T, M>
where
    M: MarketMaker + Stable,
{
    pub fn best_market_maker(&self) -> Option<&M>
    where
        T: MarketTaker,
        M: MarketMaker + Stable + Copy,
    {
        self.pools().values.values().max_by_key(|p| p.quality())
    }
}

#[derive(Copy, Clone)]
pub struct FillPreview {
    pub price: AbsolutePrice,
    pub input: u64,
}

pub fn dummy_swap<M: MarketMaker + Stable>(
    demand: u64,
    side: Side,
    maker: &M,
) -> Option<(M::StableId, FillPreview)> {
    let real_price = maker.real_price(side.wrap(demand))?;
    Some((
        maker.stable_id(),
        FillPreview {
            input: demand,
            price: real_price,
        },
    ))
}

pub fn try_optimized_swap<M: MarketMaker + Stable>(
    price: AbsolutePrice,
    demand: u64,
    side: Side,
    maker: &M,
) -> Option<(M::StableId, FillPreview)> {
    let AvailableLiquidity { input, output } = maker.available_liquidity_on_side(side.wrap(price))?;
    let absolute_price = match side {
        Side::Bid => AbsolutePrice::new(input, output)?,
        Side::Ask => AbsolutePrice::new(output, input)?,
    };
    if input > 0 && demand >= input {
        return Some((
            maker.stable_id(),
            FillPreview {
                price: absolute_price,
                input,
            },
        ));
    }
    None
}

impl<T, M> TLBState<T, M>
where
    M: Stable + Copy,
{
    pub fn preselect_market_maker(
        &self,
        price: AbsolutePrice,
        demand: u64,
        side: Side,
        optimized: bool,
    ) -> Option<(M::StableId, FillPreview)>
    where
        M: MarketMaker,
    {
        let pools = self
            .pools()
            .values
            .values()
            .filter(|pool| pool.is_active())
            .filter_map(|p| {
                if optimized {
                    try_optimized_swap(price, demand, side, p).or_else(|| dummy_swap(demand, side, p))
                } else {
                    dummy_swap(demand, side, p)
                }
            });
        match side {
            Side::Bid => pools.min_by_key(|(_, rp)| rp.price),
            Side::Ask => pools.max_by_key(|(_, rp)| rp.price),
        }
    }

    pub fn pick_maker_by_id(&mut self, pid: &M::StableId) -> Option<M>
    where
        T: MarketTaker + Ord + Copy,
    {
        self.pick_maker(|pools| pools.values.remove(pid))
    }

    /// Pick pool ensuring TLB is in proper state.
    fn pick_maker<F>(&mut self, f: F) -> Option<M>
    where
        F: FnOnce(&mut MarketMakers<M>) -> Option<M>,
        T: MarketTaker + Ord + Copy,
    {
        match self {
            // Transit into PartialPreview if state is untouched yet
            this @ TLBState::Idle(_) => {
                let mut busy_st = PartialPreviewState::new(0);
                this.move_into_partial_preview(&mut busy_st);
                let pools_preview = &mut busy_st.makers_preview;
                let result = f(pools_preview);
                mem::swap(this, &mut TLBState::PartialPreview(busy_st));
                result
            }
            TLBState::PartialPreview(busy_st) => {
                let pools_preview = &mut busy_st.makers_preview;
                f(pools_preview)
            }
            TLBState::Preview(preview_st) => {
                let pools_preview = &mut preview_st.makers_preview;
                f(pools_preview)
            }
        }
    }

    fn pools(&self) -> &MarketMakers<M> {
        match self {
            TLBState::Idle(st) => &st.makers,
            TLBState::PartialPreview(st) => &st.makers_preview,
            TLBState::Preview(st) => &st.makers_preview,
        }
    }
}

fn pick_best_fr_either<T, U>(
    active_frontier: &mut MarketTakers<T>,
    index_price: Option<AbsolutePrice>,
) -> Option<T>
where
    T: MarketTaker<U = U> + Ord + Copy,
    U: PartialOrd,
{
    let best_bid = active_frontier.bids.pop_first();
    let best_ask = active_frontier.asks.pop_first();
    match (best_bid, best_ask) {
        (Some(bid), Some(ask)) => {
            let bid_is_underpriced = index_price.map(|ip| bid.price() < ip).unwrap_or(false);
            let ask_is_overpriced = index_price.map(|ip| ask.price() > ip).unwrap_or(false);
            let bid_is_heavier = bid.weight() >= ask.weight();
            if (bid_is_heavier && !bid_is_underpriced) || ask_is_overpriced {
                active_frontier.asks.insert(ask);
                Some(bid)
            } else {
                active_frontier.bids.insert(bid);
                Some(ask)
            }
        }
        (Some(any), None) | (None, Some(any)) => Some(any),
        _ => {
            trace!(target: "state", "No best fragment");
            None
        }
    }
}

fn try_pick_fr<T, F>(active_frontier: &mut MarketTakers<T>, side: Side, test: F) -> Option<T>
where
    T: MarketTaker + Copy + Ord,
    F: FnOnce(&T) -> bool,
{
    let side = match side {
        Side::Bid => &mut active_frontier.bids,
        Side::Ask => &mut active_frontier.asks,
    };
    if let Some(best_fr) = side.pop_first() {
        if test(&best_fr) {
            return Some(best_fr);
        } else {
            side.insert(best_fr);
        }
    }
    None
}

/// Liquidity fragments spread across time axis.
#[derive(Debug, Clone, Eq, PartialEq)]
struct Chronology<T> {
    time_now: u64,
    active: MarketTakers<T>,
    inactive: BTreeMap<u64, MarketTakers<T>>,
}

impl<T> Chronology<T> {
    pub fn new(time_now: u64) -> Self {
        Self {
            time_now,
            active: MarketTakers::new(),
            inactive: BTreeMap::new(),
        }
    }
}

impl<T> Chronology<T>
where
    T: MarketTaker + TakerBehaviour + Ord + Copy,
{
    fn advance_clocks(&mut self, new_time: u64) {
        let new_slot = self
            .inactive
            .remove(&new_time)
            .unwrap_or_else(|| MarketTakers::new());
        let MarketTakers { asks, bids } = mem::replace(&mut self.active, new_slot);
        for fr in asks {
            if let Next::Succ(next_fr) = fr.with_updated_time(new_time) {
                self.active.asks.insert(next_fr);
            }
        }
        for fr in bids {
            if let Next::Succ(next_fr) = fr.with_updated_time(new_time) {
                self.active.bids.insert(next_fr);
            }
        }
        self.time_now = new_time;
    }

    fn remove_fragment(&mut self, fr: T) {
        if let Some(lower_bound) = fr.time_bounds().lower_bound() {
            if lower_bound > self.time_now {
                match self.inactive.entry(lower_bound) {
                    btree_map::Entry::Occupied(e) => {
                        match fr.side() {
                            Side::Bid => e.into_mut().bids.remove(&fr),
                            Side::Ask => e.into_mut().asks.remove(&fr),
                        };
                    }
                    btree_map::Entry::Vacant(_) => {}
                }
                return;
            }
        }
        match fr.side() {
            Side::Bid => {
                self.active.bids.remove(&fr);
            }
            Side::Ask => {
                self.active.asks.remove(&fr);
            }
        };
    }

    fn add_fragment(&mut self, fr: T) {
        match fr.time_bounds().lower_bound() {
            Some(lower_bound) if lower_bound > self.time_now => match self.inactive.entry(lower_bound) {
                btree_map::Entry::Vacant(e) => {
                    let mut fresh_fragments = MarketTakers::new();
                    fresh_fragments.insert(fr);
                    e.insert(fresh_fragments);
                }
                btree_map::Entry::Occupied(e) => {
                    e.into_mut().insert(fr);
                }
            },
            _ => {
                self.active.insert(fr);
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MarketTakers<T> {
    asks: BTreeSet<T>,
    bids: BTreeSet<T>,
}

impl<T> MarketTakers<T> {
    fn new() -> Self {
        Self {
            asks: BTreeSet::new(),
            bids: BTreeSet::new(),
        }
    }
}

impl<T> MarketTakers<T>
where
    T: MarketTaker + Ord,
{
    pub fn insert(&mut self, fr: T) {
        match fr.side() {
            Side::Bid => self.bids.insert(fr),
            Side::Ask => self.asks.insert(fr),
        };
    }

    pub fn remove(&mut self, fr: &T) {
        match fr.side() {
            Side::Bid => self.bids.remove(fr),
            Side::Ask => self.asks.remove(fr),
        };
    }

    pub fn show_state(&self) -> String
    where
        T: Display,
    {
        let asks = self
            .asks
            .iter()
            .map(|v| v.to_string())
            .fold("".to_string(), |acc, x| acc.add(format!("{}, ", x).as_str()));
        let bids = self
            .bids
            .iter()
            .map(|v| v.to_string())
            .fold("".to_string(), |acc, x| acc.add(format!("{}, ", x).as_str()));
        format!("asks: {}, bids: {}", asks, bids)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MarketMakers<M: Stable> {
    values: HashMap<M::StableId, M>,
    quality_index: BTreeMap<PoolQuality, M::StableId>,
}

impl<M: Stable> MarketMakers<M> {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            quality_index: BTreeMap::new(),
        }
    }

    pub fn show_state(&self) -> String
    where
        M::StableId: Display,
        M: Display,
    {
        self.values
            .iter()
            .map(|(k, v)| format!("{} -> {}", k, v))
            .fold("".to_string(), |acc, x| acc.add(format!("{}, ", x).as_str()))
    }
}

impl<M> MarketMakers<M>
where
    M: MarketMaker + Stable + Copy,
{
    pub fn update_pool(&mut self, pool: M) {
        if let Some(old_pool) = self.values.insert(pool.stable_id(), pool) {
            trace!(target: "state", "removing old pool {}", old_pool.stable_id());
            self.quality_index.remove(&old_pool.quality());
        }
        trace!(target: "state", "adding new pool id: {}, quality: {:?}", pool.stable_id(), pool.quality());
        self.quality_index.insert(pool.quality(), pool.stable_id());
    }
    pub fn remove_pool(&mut self, pool: M) {
        self.values.remove(&pool.stable_id());
        self.quality_index.remove(&pool.quality());
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, derive_more::Display, Serialize, Deserialize)]
#[display("LiquidityBookSize(num_active_takers= {}, num_active_makers= {}, num_idle_takers= {}, num_idle_makers= {})", num_active_takers, num_active_makers, num_idle_takers, num_idle_makers)]
pub struct LiquidityBookSize {
    pub num_active_takers: usize,
    pub num_active_makers: usize,
    pub num_idle_takers: usize,
    pub num_idle_makers: usize,
}

#[cfg(test)]
pub mod tests {
    use std::cmp::{max, Ordering};
    use std::fmt::{Debug, Display, Formatter};

    use either::Left;
    use spectrum_offchain::domain::Stable;
    use void::Void;

    use crate::execution_engine::liquidity_book::core::{Next, TerminalTake, Trans, Unit};
    use crate::execution_engine::liquidity_book::market_maker::{
        AbsoluteReserves, AvailableLiquidity, MakerBehavior, MarketMaker, SpotPrice,
    };
    use crate::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
    use crate::execution_engine::liquidity_book::ok;
    use crate::execution_engine::liquidity_book::side::{OnSide, Side};
    use crate::execution_engine::liquidity_book::state::queries::{max_by_distance_to_spot, max_by_volume};
    use crate::execution_engine::liquidity_book::state::{
        AllowedPriceRange, Chronology, IdleState, MarketMakers, PartialPreviewState, PoolQuality,
        StashingOption, TLBState,
    };
    use crate::execution_engine::liquidity_book::time::TimeBounds;
    use crate::execution_engine::liquidity_book::types::{
        AbsolutePrice, ExCostUnits, FeeAsset, InputAsset, OutputAsset,
    };
    use crate::execution_engine::types::StableId;

    #[test]
    fn tlb_lifecycle_rollback() {
        let mut idle_st: IdleState<SimpleOrderPF, SimpleCFMMPool> = IdleState {
            takers: Chronology::new(0),
            makers: MarketMakers::new(),
        };
        idle_st.add_fragment(SimpleOrderPF::new_tagged(
            "a28a8849b08a026a19b4b0dc75d1b7f79f99d9d27ef155016bb17f30",
            Side::Bid,
            25880000000,
            AbsolutePrice::new_unsafe(6470000000, 935895089),
            0,
        ));
        idle_st.add_fragment(SimpleOrderPF::new_tagged(
            "7c20ab10fdce979f02cb361837445637d42f501dfa0c5f3a4e5a2e85",
            Side::Ask,
            1000000000,
            AbsolutePrice::new_unsafe(1724137931, 250000000),
            0,
        ));
        idle_st.add_fragment(SimpleOrderPF::new_tagged(
            "418c8d82e393c1136b30dd8ad743ec8b331941f032f69dc829655fef",
            Side::Ask,
            282500001,
            AbsolutePrice::new_unsafe(1948275869, 282500001),
            0,
        ));
        idle_st.add_fragment(SimpleOrderPF::new_tagged(
            "345fe68eef783ab857427d2d058ef71242304bfd79b64ee3d532d66a",
            Side::Ask,
            2500000000,
            AbsolutePrice::new_unsafe(19841269841, 2500000000),
            0,
        ));
        let amm = SimpleCFMMPool {
            pool_id: StableId::random(),
            reserves_base: 1149079050982,
            reserves_quote: 164214639686,
            fee_num: 995,
        };
        idle_st.update_pool(amm);
        let mut st = TLBState::Idle(idle_st);

        let intact_st = st.clone();

        println!("TLB: {}", st);

        let spot_price = st.best_market_maker().map(|mm| mm.static_price());
        let price_range = st.allowed_price_range();
        let xa2 = st.pick_active_taker(|fs| {
            spot_price
                .map(|sp| max_by_distance_to_spot(fs, sp, price_range))
                .unwrap_or_else(|| max_by_volume(fs, price_range))
        });
        let x7c = st.try_pick_taker(Side::Ask, ok);
        dbg!(xa2);
        dbg!(x7c);
        st.pre_add_taker(xa2.unwrap());
        let xa2_2 = st.pick_active_taker(|fs| {
            spot_price
                .map(|sp| max_by_distance_to_spot(fs, sp, price_range))
                .unwrap_or_else(|| max_by_volume(fs, price_range))
        });
        dbg!(xa2_2);
        let pool = st.pick_maker_by_id(&amm.pool_id);
        dbg!(pool);
        st.pre_add_maker(amm);
        let x418 = st.pick_active_taker(|fs| {
            spot_price
                .map(|sp| max_by_distance_to_spot(fs, sp, price_range))
                .unwrap_or_else(|| max_by_volume(fs, price_range))
        });
        dbg!(x418);
        let pool_2 = st.pick_maker_by_id(&amm.pool_id);
        dbg!(pool_2);
        st.pre_add_maker(amm);
        let x345 = st.pick_active_taker(|fs| {
            spot_price
                .map(|sp| max_by_distance_to_spot(fs, sp, price_range))
                .unwrap_or_else(|| max_by_volume(fs, price_range))
        });
        dbg!(x345);
        st.pre_add_taker(x345.unwrap());
        st.rollback(StashingOption::Unstash);

        println!("TLB: {}", st);

        assert!(st == intact_st);
    }

    #[test]
    fn get_allowed_price_range_both_sides() {
        let st: TLBState<SimpleOrderPF, SimpleCFMMPool> = TLBState::PartialPreview(PartialPreviewState {
            takers_preview: Chronology::new(0),
            consumed_active_takers: vec![],
            stashed_active_takers: vec![
                SimpleOrderPF::new(Side::Ask, 0, AbsolutePrice::new_unsafe(1, 2), 1100000, 900000),
                SimpleOrderPF::new(Side::Ask, 0, AbsolutePrice::new_unsafe(1, 3), 1100000, 900000),
                SimpleOrderPF::new(Side::Bid, 0, AbsolutePrice::new_unsafe(2, 3), 1100000, 900000),
                SimpleOrderPF::new(Side::Bid, 0, AbsolutePrice::new_unsafe(1, 3), 1100000, 900000),
            ],
            makers_intact: MarketMakers::new(),
            makers_preview: MarketMakers::new(),
        });
        let range = st.allowed_price_range();
        let expected_range = AllowedPriceRange {
            max_ask_price: Some(AbsolutePrice::new_unsafe(1, 3)),
            min_bid_price: Some(AbsolutePrice::new_unsafe(2, 3)),
        };
        assert_eq!(range, expected_range);
    }

    #[test]
    fn get_allowed_price_range_one_side() {
        let st: TLBState<SimpleOrderPF, SimpleCFMMPool> = TLBState::PartialPreview(PartialPreviewState {
            takers_preview: Chronology::new(0),
            consumed_active_takers: vec![],
            stashed_active_takers: vec![
                SimpleOrderPF::new(Side::Ask, 0, AbsolutePrice::new_unsafe(1, 2), 1100000, 900000),
                SimpleOrderPF::new(Side::Ask, 0, AbsolutePrice::new_unsafe(1, 3), 1100000, 900000),
            ],
            makers_intact: MarketMakers::new(),
            makers_preview: MarketMakers::new(),
        });
        let range = st.allowed_price_range();
        let expected_range = AllowedPriceRange {
            max_ask_price: Some(AbsolutePrice::new_unsafe(1, 3)),
            min_bid_price: None,
        };
        assert_eq!(range, expected_range);
    }

    #[test]
    fn add_inactive_fragment() {
        let time_now = 1000u64;
        let ord = SimpleOrderPF::default_with_bounds(TimeBounds::After(time_now + 100));
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(ord);
        assert_eq!(TLBState::Idle(s0).pick_best_fr_either(None), None);
    }

    #[test]
    fn pop_active_fragment() {
        let time_now = 1000u64;
        let ord = SimpleOrderPF::default_with_bounds(TimeBounds::None);
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(ord);
        let mut s0_wrapped = TLBState::Idle(s0);
        assert_eq!(s0_wrapped.pick_best_fr_either(None), Some(ord));
        assert_eq!(s0_wrapped.pick_best_fr_either(None), None);
    }

    #[test]
    fn fragment_activation() {
        let time_now = 1000u64;
        let delta = 100u64;
        let ord = SimpleOrderPF::default_with_bounds(TimeBounds::After(time_now + delta));
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(ord);
        assert_eq!(TLBState::Idle(s0.clone()).pick_best_fr_either(None), None);
        s0.takers.advance_clocks(time_now + delta);
        assert_eq!(TLBState::Idle(s0).pick_best_fr_either(None), Some(ord));
    }

    #[test]
    fn fragment_deactivation() {
        let time_now = 1000u64;
        let delta = 100u64;
        let ord = SimpleOrderPF::default_with_bounds(TimeBounds::Until(time_now + delta));
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(ord);
        assert_eq!(TLBState::Idle(s0.clone()).pick_best_fr_either(None), Some(ord));
        s0.takers.advance_clocks(time_now + delta + 1);
        assert_eq!(TLBState::Idle(s0).pick_best_fr_either(None), None);
    }

    #[test]
    fn choose_best_fragment_bid_is_underpriced() {
        let time_now = 1000u64;
        let index_price = AbsolutePrice::new_unsafe(1, 35);
        let ask = SimpleOrderPF::new(Side::Ask, 1000, index_price, 1100000, 900000);
        let bid = SimpleOrderPF::new(Side::Bid, 1000, AbsolutePrice::new_unsafe(1, 40), 1100000, 900000);
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(ask);
        s0.takers.add_fragment(bid);
        assert_eq!(
            TLBState::Idle(s0).pick_best_fr_either(Some(index_price)),
            Some(ask)
        );
    }

    #[test]
    fn choose_best_fragment_ask_is_overpriced() {
        let time_now = 1000u64;
        let index_price = AbsolutePrice::new_unsafe(1, 35);
        let ask = SimpleOrderPF::new(Side::Ask, 1000, AbsolutePrice::new_unsafe(1, 30), 1100000, 900000);
        let bid = SimpleOrderPF::new(Side::Bid, 1000, index_price, 1100000, 900000);
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(ask);
        s0.takers.add_fragment(bid);
        assert_eq!(
            TLBState::Idle(s0).pick_best_fr_either(Some(index_price)),
            Some(bid)
        );
    }

    #[test]
    fn choose_best_fragment_both_orders_price_is_off() {
        let time_now = 1000u64;
        let index_price = AbsolutePrice::new_unsafe(1, 35);
        let ask = SimpleOrderPF::new(Side::Ask, 1000, AbsolutePrice::new_unsafe(1, 30), 1100000, 900000);
        let bid = SimpleOrderPF::new(Side::Bid, 1000, AbsolutePrice::new_unsafe(1, 40), 1100000, 900000);
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(ask);
        s0.takers.add_fragment(bid);
        assert_eq!(
            TLBState::Idle(s0).pick_best_fr_either(Some(index_price)),
            Some(bid)
        );
    }

    #[test]
    fn settled_state_to_preview_active_fr() {
        let time_now = 1000u64;
        let delta = 100u64;
        let o1 = SimpleOrderPF::default_with_bounds(TimeBounds::Until(time_now + delta));
        let o2 = SimpleOrderPF::default_with_bounds(TimeBounds::None);
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(o1);
        let s0_copy = s0.clone();
        let mut state = TLBState::Idle(s0);
        state.pre_add_taker(o2);
        match state {
            TLBState::Preview(st) => {
                assert_eq!(st.takers_intact, s0_copy.takers);
                let preview = st.active_takers_preview;
                assert!(preview.bids.contains(&o1) || preview.asks.contains(&o1));
                assert!(preview.bids.contains(&o2) || preview.asks.contains(&o2));
                dbg!(preview);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn settled_state_to_preview_inactive_fr() {
        let time_now = 1000u64;
        let delta = 100u64;
        let o1 = SimpleOrderPF::default_with_bounds(TimeBounds::Until(time_now + delta));
        let o2 = SimpleOrderPF::default_with_bounds(TimeBounds::After(time_now + delta));
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(o1);
        let s0_copy = s0.clone();
        let mut state = TLBState::Idle(s0);
        state.pre_add_taker(o2);
        match state {
            TLBState::Preview(st) => {
                assert_eq!(st.takers_intact, s0_copy.takers);
                assert_eq!(
                    st.inactive_takers_changeset,
                    vec![(o2.bounds.lower_bound().unwrap(), o2)]
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn commit_preview_changes() {
        let time_now = 1000u64;
        let delta = 100u64;
        let o1 = SimpleOrderPF::default_with_bounds(TimeBounds::Until(time_now + delta));
        let o2 = SimpleOrderPF::default_with_bounds(TimeBounds::None);
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(o1);
        let _s0_copy = s0.clone();
        let mut state = TLBState::Idle(s0);
        state.pre_add_taker(o2);
        match state {
            TLBState::Preview(mut s1) => {
                let s1_copy = s1.clone();
                let s2 = s1.commit();
                for (t, fr) in s1_copy.inactive_takers_changeset {
                    assert!(s2
                        .takers
                        .inactive
                        .get(&t)
                        .map(|frs| frs.asks.contains(&fr) || frs.bids.contains(&fr))
                        .unwrap_or(false));
                }
                for fr in &s1_copy.active_takers_preview.bids {
                    assert!(s2.takers.active.bids.contains(&fr))
                }
                for fr in &s1_copy.active_takers_preview.asks {
                    assert!(s2.takers.active.asks.contains(&fr))
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn rollback_preview_changes_deletion() {
        let time_now = 1000u64;
        let delta = 100u64;
        let o1 = SimpleOrderPF::default_with_bounds(TimeBounds::Until(time_now + delta));
        let o2 = SimpleOrderPF::default_with_bounds(TimeBounds::None);
        let o3 = SimpleOrderPF::default_with_bounds(TimeBounds::None);
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(o1);
        s0.takers.add_fragment(o2);
        let s0_copy = s0.clone();
        let mut state = TLBState::Idle(s0);
        // One new fragment added into the preview.
        state.pre_add_taker(o3);
        // One old fragment removed from the preview.
        assert!(matches!(state.pick_best_fr_either(None), Some(_)));
        match state {
            TLBState::Preview(mut s1) => {
                if let Left(s2) = s1.rollback(StashingOption::Unstash) {
                    assert_eq!(s2.takers, s0_copy.takers);
                    assert_eq!(s2.makers, s0_copy.makers);
                } else {
                    panic!()
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn rollback_part_preview_changes_deletion() {
        let time_now = 1000u64;
        let delta = 100u64;
        let o1 = SimpleOrderPF::default_with_bounds(TimeBounds::Until(time_now + delta));
        let o2 = SimpleOrderPF::default_with_bounds(TimeBounds::None);
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(o1);
        s0.takers.add_fragment(o2);
        let s0_copy = s0.clone();
        let mut state = TLBState::Idle(s0);
        // One old fragment removed from the preview.
        assert!(matches!(state.pick_best_fr_either(None), Some(_)));
        match state {
            TLBState::PartialPreview(mut s1) => {
                if let Left(s2) = s1.rollback(StashingOption::Unstash) {
                    assert_eq!(s2.takers, s0_copy.takers);
                    assert_eq!(s2.makers, s0_copy.makers);
                } else {
                    panic!()
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn stash_unstash() {
        let time_now = 1000u64;
        let o2 = SimpleOrderPF::default_with_bounds(TimeBounds::None);
        let p0 = SimpleCFMMPool {
            pool_id: StableId::random(),
            reserves_base: 0,
            reserves_quote: 0,
            fee_num: 0,
        };
        let mut s0 = IdleState::<_, SimpleCFMMPool>::new(time_now);
        s0.takers.add_fragment(o2);
        s0.makers.update_pool(p0);
        let mut state = TLBState::Idle(s0);
        state.commit();
        assert_eq!(state.pick_best_fr_either(None), Some(o2));
        state.rollback(StashingOption::Stash(vec![o2]));
        assert_eq!(state.pools().values.get(&p0.pool_id).copied(), Some(p0));
        assert_eq!(state.pick_best_fr_either(None), None);
        state.rollback(StashingOption::Unstash);
        assert_eq!(state.pick_best_fr_either(None), Some(o2));
        assert_eq!(state.pools().values.get(&p0.pool_id).copied(), Some(p0));
    }

    /// Order that supports partial filling.
    #[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
    pub struct SimpleOrderPF {
        pub source: StableId,
        pub side: Side,
        pub input: u64,
        pub accumulated_output: u64,
        pub min_marginal_output: u64,
        pub price: AbsolutePrice,
        pub fee: u64,
        pub ex_budget: u64,
        pub cost_hint: ExCostUnits,
        pub bounds: TimeBounds<u64>,
    }

    impl Stable for SimpleOrderPF {
        type StableId = StableId;
        fn stable_id(&self) -> Self::StableId {
            self.source
        }
        fn is_quasi_permanent(&self) -> bool {
            true
        }
    }

    impl Display for SimpleOrderPF {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.write_str(&*format!(
                "Ord(input={}, price={}, side={}, fee={})",
                self.input, self.price, self.side, self.fee
            ))
        }
    }

    impl PartialOrd for SimpleOrderPF {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for SimpleOrderPF {
        fn cmp(&self, other: &Self) -> Ordering {
            self.price.cmp(&other.price).then(self.source.cmp(&other.source))
        }
    }

    impl SimpleOrderPF {
        pub fn new(side: Side, input: u64, price: AbsolutePrice, ex_budget: u64, fee: u64) -> Self {
            Self {
                source: StableId::random(),
                side,
                input,
                accumulated_output: 0,
                min_marginal_output: 0,
                price,
                fee,
                ex_budget,
                cost_hint: 10,
                bounds: TimeBounds::None,
            }
        }

        pub fn new_tagged(tag: &str, side: Side, input: u64, price: AbsolutePrice, fee: u64) -> Self {
            Self {
                source: StableId::from(<[u8; 28]>::try_from(hex::decode(tag).unwrap()).unwrap()),
                side,
                input,
                accumulated_output: 0,
                min_marginal_output: 0,
                price,
                fee,
                ex_budget: 0,
                cost_hint: 10,
                bounds: TimeBounds::None,
            }
        }

        pub fn make(
            side: Side,
            input: u64,
            price: AbsolutePrice,
            fee: u64,
            accumulated_output: u64,
            min_marginal_output: u64,
        ) -> Self {
            Self {
                source: StableId::random(),
                side,
                input,
                accumulated_output,
                min_marginal_output,
                price,
                fee,
                ex_budget: 0,
                cost_hint: 10,
                bounds: TimeBounds::None,
            }
        }
        pub fn default_with_bounds(bounds: TimeBounds<u64>) -> Self {
            Self {
                source: StableId::random(),
                side: Side::Ask,
                input: 1000_000_000,
                accumulated_output: 0,
                min_marginal_output: 0,
                price: AbsolutePrice::new_unsafe(1, 100),
                fee: 100,
                ex_budget: 0,
                cost_hint: 0,
                bounds,
            }
        }
    }

    impl MarketTaker for SimpleOrderPF {
        type U = u64;

        fn side(&self) -> Side {
            self.side
        }

        fn input(&self) -> u64 {
            self.input
        }

        fn output(&self) -> OutputAsset<u64> {
            self.accumulated_output
        }

        fn price(&self) -> AbsolutePrice {
            self.price
        }

        fn marginal_cost_hint(&self) -> ExCostUnits {
            self.cost_hint
        }

        fn time_bounds(&self) -> TimeBounds<u64> {
            self.bounds
        }

        fn operator_fee(&self, input_consumed: InputAsset<u64>) -> FeeAsset<u64> {
            self.fee * input_consumed / self.input
        }

        fn min_marginal_output(&self) -> OutputAsset<u64> {
            self.min_marginal_output
        }

        fn fee(&self) -> FeeAsset<u64> {
            self.fee
        }

        fn budget(&self) -> FeeAsset<u64> {
            self.ex_budget
        }

        fn consumable_budget(&self) -> FeeAsset<u64> {
            self.ex_budget
        }
    }

    impl TakerBehaviour for SimpleOrderPF {
        fn with_updated_time(self, time: u64) -> Next<Self, Unit> {
            if self.bounds.contain(&time) {
                Next::Succ(self)
            } else {
                Next::Term(Unit)
            }
        }

        fn with_applied_trade(
            mut self,
            removed_input: InputAsset<u64>,
            added_output: OutputAsset<u64>,
        ) -> Next<Self, TerminalTake> {
            self.input -= removed_input;
            self.accumulated_output += added_output;
            self.try_terminate()
        }

        fn with_budget_corrected(mut self, delta: i64) -> (i64, Self) {
            let budget_remainder = self.ex_budget as i64;
            let corrected_remainder = budget_remainder + delta;
            let updated_budget_remainder = max(corrected_remainder, 0);
            let real_delta = budget_remainder - updated_budget_remainder;
            self.ex_budget = updated_budget_remainder as u64;
            (real_delta, self)
        }

        fn with_fee_charged(mut self, fee: u64) -> Self {
            self.fee -= fee;
            self
        }

        fn with_output_added(mut self, added_output: u64) -> Self {
            self.accumulated_output += added_output;
            self
        }

        fn try_terminate(self) -> Next<Self, TerminalTake> {
            if self.input > 0 {
                Next::Succ(self)
            } else {
                Next::Term(TerminalTake {
                    remaining_input: self.input,
                    accumulated_output: self.accumulated_output,
                    remaining_fee: self.fee,
                    remaining_budget: self.ex_budget,
                })
            }
        }
    }

    #[derive(Copy, Clone, PartialEq, Eq, Hash)]
    pub struct SimpleCFMMPool {
        pub pool_id: StableId,
        pub reserves_base: u64,
        pub reserves_quote: u64,
        pub fee_num: u64,
    }

    impl Display for SimpleCFMMPool {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.write_str(&*format!("Pool(price={})", self.static_price()))
        }
    }

    impl Debug for SimpleCFMMPool {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.write_str(&*self.to_string())
        }
    }

    impl Stable for SimpleCFMMPool {
        type StableId = StableId;
        fn stable_id(&self) -> Self::StableId {
            self.pool_id
        }
        fn is_quasi_permanent(&self) -> bool {
            true
        }
    }

    impl MakerBehavior for SimpleCFMMPool {
        fn swap(mut self, input: OnSide<u64>) -> Next<Self, Void> {
            let result = match input {
                OnSide::Bid(quote_input) => {
                    let base_output =
                        ((self.reserves_base as u128) * (quote_input as u128) * (self.fee_num as u128)
                            / ((self.reserves_quote as u128) * 1000u128
                                + (quote_input as u128) * (self.fee_num as u128)))
                            as u64;
                    self.reserves_quote += quote_input;
                    self.reserves_base -= base_output;
                    self
                }
                OnSide::Ask(base_input) => {
                    let quote_output =
                        ((self.reserves_quote as u128) * (base_input as u128) * (self.fee_num as u128)
                            / ((self.reserves_base as u128) * 1000u128
                                + (base_input as u128) * (self.fee_num as u128)))
                            as u64;
                    self.reserves_base += base_input;
                    self.reserves_quote -= quote_output;
                    self
                }
            };
            Next::Succ(result)
        }
    }

    impl MarketMaker for SimpleCFMMPool {
        type U = u64;

        fn static_price(&self) -> SpotPrice {
            AbsolutePrice::new_unsafe(self.reserves_quote, self.reserves_base).into()
        }

        fn real_price(&self, input: OnSide<u64>) -> Option<AbsolutePrice> {
            match input {
                OnSide::Bid(quote_input) => {
                    let result_pool = self.swap(OnSide::Bid(quote_input));
                    let trans = Trans::new(*self, result_pool);
                    let base_output = trans.loss().map(|r| r.unwrap()).unwrap_or(0);
                    AbsolutePrice::new(quote_input, base_output)
                }
                OnSide::Ask(base_input) => {
                    let result_pool = self.swap(OnSide::Ask(base_input));
                    let trans = Trans::new(*self, result_pool);
                    let quote_output = trans.loss().map(|r| r.unwrap()).unwrap_or(0);
                    AbsolutePrice::new(quote_output, base_input)
                }
            }
        }

        fn quality(&self) -> PoolQuality {
            PoolQuality::from(0u128)
        }

        fn liquidity(&self) -> AbsoluteReserves {
            AbsoluteReserves {
                base: self.reserves_base,
                quote: self.reserves_quote,
            }
        }

        fn available_liquidity_on_side(
            &self,
            worst_price: OnSide<AbsolutePrice>,
        ) -> Option<AvailableLiquidity> {
            None
        }

        fn estimated_trade(&self, input: OnSide<u64>) -> Option<AvailableLiquidity> {
            None
        }

        fn marginal_cost_hint(&self) -> Self::U {
            10
        }

        fn is_active(&self) -> bool {
            // SimpleCFMMPool used only for tests
            true
        }
    }
}
