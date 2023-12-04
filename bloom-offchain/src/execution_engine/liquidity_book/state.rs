use std::collections::hash_map::Entry;
use std::collections::{btree_map, BTreeMap, BTreeSet, HashMap};
use std::fmt::Debug;
use std::mem;

use spectrum_offchain::data::Has;

use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use crate::execution_engine::liquidity_book::side::{Side, SideMarker};
use crate::execution_engine::liquidity_book::types::Price;
use crate::execution_engine::SourceId;

#[derive(Debug, Clone, Eq, PartialEq)]
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

impl<Fr, Pl> SettledState<Fr, Pl>
where
    Fr: Fragment + OrderState + Ord + Copy,
    Pl: QualityMetric + Has<SourceId> + Copy,
{
    pub fn advance_clocks(&mut self, new_time: u64) {
        self.fragments.advance_clocks(new_time)
    }

    pub fn add_fragment(&mut self, fr: Fr) {
        self.fragments.add_fragment(fr);
    }

    pub fn remove_fragment(&mut self, source: SourceId) {
        self.fragments.remove_fragment(source);
    }

    pub fn update_pool(&mut self, pool: Pl) {
        self.pools.update_pool(pool);
    }
}

#[derive(Debug, Clone)]
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
    pools_preview: Option<Pools<Pl>>,
}

impl<Fr, Pl> PreviewState<Fr, Pl>
where
    Fr: Fragment + Ord,
{
    pub fn new(time_now: u64) -> Self {
        Self {
            fragments_intact: Chronology::new(time_now),
            active_fragments_preview: None,
            inactive_fragments_changeset: vec![],
            pools_intact: Pools::new(),
            pools_preview: None,
        }
    }

    /// Commit preview changes.
    pub fn commit(&mut self) -> SettledState<Fr, Pl> {
        // Commit pools preview if available.
        if let Some(pools_preview) = &mut self.pools_preview {
            mem::swap(&mut self.pools_intact, pools_preview);
        }
        // Commit active fragments preview if available.
        if let Some(frs_preview) = &mut self.active_fragments_preview {
            mem::swap(&mut self.fragments_intact.active, frs_preview);
        }
        // Commit inactive fragments.
        while let Some((t, fr)) = self.inactive_fragments_changeset.pop() {
            match self.fragments_intact.inactive.entry(t) {
                btree_map::Entry::Vacant(entry) => {
                    let mut frs = Fragments::new();
                    frs.insert(fr);
                    entry.insert(frs);
                }
                btree_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().insert(fr);
                }
            }
        }
        self.move_into_settled()
    }

    /// Discard preview changes.
    pub fn rollback(&mut self) -> SettledState<Fr, Pl> {
        self.move_into_settled()
    }

    /// Move intact regions into settled state.
    fn move_into_settled(&mut self) -> SettledState<Fr, Pl> {
        let mut fresh_settled_st = SettledState::new(self.fragments_intact.time_now);
        mem::swap(&mut fresh_settled_st.fragments, &mut self.fragments_intact);
        mem::swap(&mut fresh_settled_st.pools, &mut self.pools_intact);
        fresh_settled_st
    }
}

#[derive(Debug, Clone)]
pub enum TLBState<Fr, Pl> {
    Settled(SettledState<Fr, Pl>),
    Preview(PreviewState<Fr, Pl>),
}

impl<Fr, Pl> TLBState<Fr, Pl>
where
    Fr: Fragment + Ord + Copy,
    Pl: QualityMetric + Has<SourceId> + Copy,
{
    pub fn active_fragments(&self) -> &Fragments<Fr> {
        match self {
            TLBState::Settled(st) => &st.fragments.active,
            TLBState::Preview(st) => match &st.active_fragments_preview {
                None => &st.fragments_intact.active,
                Some(active) => active,
            },
        }
    }

    pub fn active_fragments_mut(&mut self) -> &mut Fragments<Fr> {
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
            TLBState::Preview(st) => match &st.pools_preview {
                None => &st.pools_intact,
                Some(pools) => pools,
            },
        }
    }

    pub fn pools_mut(&mut self) -> &mut Pools<Pl> {
        match self {
            TLBState::Settled(st) => &mut st.pools,
            TLBState::Preview(st) => match &mut st.pools_preview {
                None => &mut st.pools_intact,
                Some(pools) => pools,
            },
        }
    }

    pub fn pre_add_fragment(&mut self, fr: Fr) {
        let time = self.current_time();
        match (self, fr.time_bounds().lower_bound()) {
            // We have to transit to preview state.
            (this @ TLBState::Settled(_), lower_bound) => {
                let mut preview_st = PreviewState::new(time);
                match this {
                    TLBState::Settled(st) => {
                        mem::swap(&mut preview_st.fragments_intact, &mut st.fragments);
                        mem::swap(&mut preview_st.pools_intact, &mut st.pools);
                        match lower_bound {
                            // If `fr` is inactive we avoid cloning active frontier.
                            Some(lower_bound) if lower_bound > time => {
                                preview_st.inactive_fragments_changeset.push((lower_bound, fr));
                            }
                            // Otherwise we have to create a copy of active frontier and add new `fr` to it.
                            _ => {
                                let mut active_fragments = preview_st.fragments_intact.active.clone();
                                active_fragments.insert(fr);
                                preview_st.active_fragments_preview.replace(active_fragments);
                            }
                        }
                    }
                    TLBState::Preview(_) => unreachable!(),
                }
                mem::swap(this, &mut TLBState::Preview(preview_st));
            }
            (TLBState::Preview(ref mut preview_st), lower_bound) => match lower_bound {
                Some(lb) if lb > time => preview_st.inactive_fragments_changeset.push((lb, fr)),
                _ => match &mut preview_st.active_fragments_preview {
                    Some(active_preview) => active_preview.insert(fr),
                    this_active_preview => {
                        let mut active_preview = preview_st.fragments_intact.active.clone();
                        active_preview.insert(fr);
                        this_active_preview.replace(active_preview);
                    }
                },
            },
        }
    }

    pub fn pre_add_pool(&mut self, pool: Pl) {
        match self {
            this @ TLBState::Settled(_) => {
                let mut preview_st = PreviewState::new(0);
                match this {
                    TLBState::Settled(st) => {
                        // Move initial views into fresh preview state.
                        mem::swap(&mut preview_st.fragments_intact, &mut st.fragments);
                        mem::swap(&mut preview_st.pools_intact, &mut st.pools);
                        // Copy initial pools view ..
                        let mut pools_preview = preview_st.pools_intact.clone();
                        // .. and update it with new pool state.
                        pools_preview.update_pool(pool);
                        mem::swap(&mut preview_st.pools_preview, &mut Some(pools_preview));
                    }
                    TLBState::Preview(_) => unreachable!(),
                }
                mem::swap(this, &mut TLBState::Preview(preview_st));
            }
            TLBState::Preview(ref mut state) => match &mut state.pools_preview {
                Some(pools_preview) => pools_preview.update_pool(pool),
                this_pools_preview => {
                    let mut pools_preview = state.pools_intact.clone();
                    pools_preview.update_pool(pool);
                    mem::swap(this_pools_preview, &mut Some(pools_preview));
                }
            },
        }
    }

    fn current_time(&self) -> u64 {
        match self {
            TLBState::Settled(st) => st.fragments.time_now,
            TLBState::Preview(st) => st.fragments_intact.time_now,
        }
    }
}

/// Liquidity fragments spread across time axis.
#[derive(Debug, Clone, Eq, PartialEq)]
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

    fn remove_fragment(&mut self, source: SourceId) {
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
        match fr.time_bounds().lower_bound() {
            Some(lower_bound) if lower_bound > self.time_now => match self.inactive.entry(lower_bound) {
                btree_map::Entry::Vacant(e) => {
                    let mut fresh_fragments = Fragments::new();
                    fresh_fragments.insert(fr);
                    e.insert(fresh_fragments);
                }
                btree_map::Entry::Occupied(e) => {
                    e.into_mut().insert(fr);
                }
            },
            _ => self.active.insert(fr),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
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
            SideMarker::Ask => self.asks.insert(fr),
        };
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
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

impl<Pl> Pools<Pl>
where
    Pl: QualityMetric + Has<SourceId> + Copy,
{
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

#[cfg(test)]
pub mod tests {
    use std::cmp::Ordering;

    use cml_core::Slot;
    use num_rational::Ratio;
    use type_equalities::IsEqual;

    use spectrum_offchain::data::Has;

    use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
    use crate::execution_engine::liquidity_book::pool::Pool;
    use crate::execution_engine::liquidity_book::side::{Side, SideMarker};
    use crate::execution_engine::liquidity_book::state::{
        PoolQuality, QualityMetric, SettledState, TLBState,
    };
    use crate::execution_engine::liquidity_book::time::TimeBounds;
    use crate::execution_engine::liquidity_book::types::{ExecutionCost, Price};
    use crate::execution_engine::SourceId;

    #[test]
    fn add_inactive_fragment() {
        let time_now = 1000u64;
        let ord = SimpleOrderPF::default_with_bounds(TimeBounds::After(time_now + 100));
        let mut s0 = SettledState::<_, SimpleCFMMPool>::new(time_now);
        s0.fragments.add_fragment(ord);
        assert_eq!(TLBState::Settled(s0).active_fragments_mut().pick_either(), None);
    }

    #[test]
    fn fragment_activation() {
        let time_now = 1000u64;
        let delta = 100u64;
        let ord = SimpleOrderPF::default_with_bounds(TimeBounds::After(time_now + delta));
        let mut s0 = SettledState::<_, SimpleCFMMPool>::new(time_now);
        s0.fragments.add_fragment(ord);
        assert_eq!(
            TLBState::Settled(s0.clone()).active_fragments_mut().pick_either(),
            None
        );
        s0.fragments.advance_clocks(time_now + delta);
        assert_eq!(
            TLBState::Settled(s0).active_fragments_mut().pick_either(),
            Some(ord)
        );
    }

    #[test]
    fn fragment_deactivation() {
        let time_now = 1000u64;
        let delta = 100u64;
        let ord = SimpleOrderPF::default_with_bounds(TimeBounds::Until(time_now + delta));
        let mut s0 = SettledState::<_, SimpleCFMMPool>::new(time_now);
        s0.fragments.add_fragment(ord);
        assert_eq!(
            TLBState::Settled(s0.clone()).active_fragments_mut().pick_either(),
            Some(ord)
        );
        s0.fragments.advance_clocks(time_now + delta);
        assert_eq!(TLBState::Settled(s0).active_fragments_mut().pick_either(), None);
    }

    #[test]
    fn settled_state_to_preview_active_fr() {
        let time_now = 1000u64;
        let delta = 100u64;
        let o1 = SimpleOrderPF::default_with_bounds(TimeBounds::Until(time_now + delta));
        let o2 = SimpleOrderPF::default_with_bounds(TimeBounds::None);
        let mut s0 = SettledState::<_, SimpleCFMMPool>::new(time_now);
        s0.fragments.add_fragment(o1);
        let s0_copy = s0.clone();
        let mut state = TLBState::Settled(s0);
        state.pre_add_fragment(o2);
        match state {
            TLBState::Preview(st) => {
                assert_eq!(st.fragments_intact, s0_copy.fragments);
                match st.active_fragments_preview {
                    Some(preview) => {
                        assert!(preview.bids.contains(&o1) || preview.asks.contains(&o1));
                        assert!(preview.bids.contains(&o2) || preview.asks.contains(&o2));
                        dbg!(preview);
                    }
                    None => panic!(),
                }
            }
            TLBState::Settled(_) => panic!(),
        }
    }

    #[test]
    fn settled_state_to_preview_inactive_fr() {
        let time_now = 1000u64;
        let delta = 100u64;
        let o1 = SimpleOrderPF::default_with_bounds(TimeBounds::Until(time_now + delta));
        let o2 = SimpleOrderPF::default_with_bounds(TimeBounds::After(time_now + delta));
        let mut s0 = SettledState::<_, SimpleCFMMPool>::new(time_now);
        s0.fragments.add_fragment(o1);
        let s0_copy = s0.clone();
        let mut state = TLBState::Settled(s0);
        state.pre_add_fragment(o2);
        match state {
            TLBState::Preview(st) => {
                assert_eq!(st.fragments_intact, s0_copy.fragments);
                assert_eq!(st.active_fragments_preview, None);
                assert_eq!(
                    st.inactive_fragments_changeset,
                    vec![(o2.bounds.lower_bound().unwrap(), o2)]
                );
            }
            TLBState::Settled(_) => panic!(),
        }
    }

    #[test]
    fn commit_preview_changes() {
        let time_now = 1000u64;
        let delta = 100u64;
        let o1 = SimpleOrderPF::default_with_bounds(TimeBounds::Until(time_now + delta));
        let o2 = SimpleOrderPF::default_with_bounds(TimeBounds::None);
        let mut s0 = SettledState::<_, SimpleCFMMPool>::new(time_now);
        s0.fragments.add_fragment(o1);
        let s0_copy = s0.clone();
        let mut state = TLBState::Settled(s0);
        state.pre_add_fragment(o2);
        match state {
            TLBState::Preview(mut s1) => {
                let s1_copy = s1.clone();
                let s2 = s1.commit();
                for (t, fr) in s1_copy.inactive_fragments_changeset {
                    assert!(s2
                        .fragments
                        .inactive
                        .get(&t)
                        .map(|frs| frs.asks.contains(&fr) || frs.bids.contains(&fr))
                        .unwrap_or(false));
                }
                for fr in &s1_copy.active_fragments_preview.as_ref().unwrap().bids {
                    assert!(s2.fragments.active.bids.contains(&fr))
                }
                for fr in &s1_copy.active_fragments_preview.as_ref().unwrap().asks {
                    assert!(s2.fragments.active.asks.contains(&fr))
                }
            }
            TLBState::Settled(_) => panic!(),
        }
    }

    #[test]
    fn rollback_preview_changes() {
        let time_now = 1000u64;
        let delta = 100u64;
        let o1 = SimpleOrderPF::default_with_bounds(TimeBounds::Until(time_now + delta));
        let o2 = SimpleOrderPF::default_with_bounds(TimeBounds::None);
        let mut s0 = SettledState::<_, SimpleCFMMPool>::new(time_now);
        s0.fragments.add_fragment(o1);
        let s0_copy = s0.clone();
        let mut state = TLBState::Settled(s0);
        state.pre_add_fragment(o2);
        match state {
            TLBState::Preview(mut s1) => {
                let s2 = s1.rollback();
                assert_eq!(s2.fragments, s0_copy.fragments);
                assert_eq!(s2.pools, s0_copy.pools);
            }
            TLBState::Settled(_) => panic!(),
        }
    }

    /// Order that supports partial filling.
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct SimpleOrderPF {
        pub source: SourceId,
        pub side: SideMarker,
        pub input: u64,
        pub accumulated_output: u64,
        pub price: Price,
        pub fee: u64,
        pub cost_hint: ExecutionCost,
        pub bounds: TimeBounds<Slot>,
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
        pub fn default_with_bounds(bounds: TimeBounds<u64>) -> Self {
            Self {
                source: SourceId::random(),
                side: SideMarker::Ask,
                input: 1000_000_000,
                accumulated_output: 0,
                price: Ratio::new(1, 100),
                fee: 100,
                cost_hint: 0,
                bounds,
            }
        }
    }

    impl Fragment for SimpleOrderPF {
        fn side(&self) -> SideMarker {
            self.side
        }

        fn input(&self) -> u64 {
            self.input
        }

        fn price(&self) -> Price {
            self.price
        }

        fn weight(&self) -> u64 {
            self.fee
        }

        fn cost_hint(&self) -> ExecutionCost {
            self.cost_hint
        }

        fn time_bounds(&self) -> TimeBounds<Slot> {
            self.bounds
        }
    }

    impl OrderState for SimpleOrderPF {
        fn source(&self) -> SourceId {
            self.source
        }

        fn with_updated_time(self, time: u64) -> StateTrans<Self> {
            if self.bounds.contain(&time) {
                StateTrans::Active(self)
            } else {
                StateTrans::EOL
            }
        }

        fn with_updated_liquidity(mut self, removed_input: u64, added_output: u64) -> StateTrans<Self> {
            self.input -= removed_input;
            self.accumulated_output += added_output;
            if self.input > 0 {
                StateTrans::Active(self)
            } else {
                StateTrans::EOL
            }
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct SimpleCFMMPool {
        pub state_ver: [u8; 32],
        pub reserves_base: u64,
        pub reserves_quote: u64,
        pub fee_num: u64,
    }

    impl QualityMetric for SimpleCFMMPool {
        fn quality(&self) -> PoolQuality {
            PoolQuality(
                Ratio::new(self.reserves_quote as u128, self.reserves_base as u128),
                self.reserves_quote + self.reserves_base,
            )
        }
    }

    impl Has<SourceId> for SimpleCFMMPool {
        fn get<U: IsEqual<SourceId>>(&self) -> SourceId {
            SourceId(self.state_ver)
        }
    }

    impl Pool for SimpleCFMMPool {
        fn static_price(&self) -> Price {
            Ratio::new(self.reserves_quote as u128, self.reserves_base as u128)
        }

        fn real_price(&self, input: Side<u64>) -> Price {
            match input {
                Side::Bid(quote_input) => {
                    let (base_output, _) = self.swap(Side::Bid(quote_input));
                    Ratio::new(quote_input as u128, base_output as u128)
                }
                Side::Ask(base_input) => {
                    let (quote_output, _) = self.swap(Side::Ask(base_input));
                    Ratio::new(quote_output as u128, base_input as u128)
                }
            }
        }

        fn swap(mut self, input: Side<u64>) -> (u64, Self) {
            match input {
                Side::Bid(quote_input) => {
                    let base_output =
                        ((self.reserves_base as u128) * (quote_input as u128) * (self.fee_num as u128)
                            / ((self.reserves_quote as u128) * 1000u128
                                + (quote_input as u128) * (self.fee_num as u128)))
                            as u64;
                    self.reserves_quote += quote_input;
                    self.reserves_base -= base_output;
                    (base_output, self)
                }
                Side::Ask(base_input) => {
                    let quote_output =
                        ((self.reserves_quote as u128) * (base_input as u128) * (self.fee_num as u128)
                            / ((self.reserves_base as u128) * 1000u128
                                + (base_input as u128) * (self.fee_num as u128)))
                            as u64;
                    self.reserves_base += base_input;
                    self.reserves_quote -= quote_output;
                    (quote_output, self)
                }
            }
        }
    }
}
