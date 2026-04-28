use algebra_core::monoid::Monoid;
use bloom_offchain::execution_engine::liquidity_book::config::ExecutionConfig;
use bloom_offchain::execution_engine::liquidity_book::core::{
    ExecutionEvent, MakeInProgress, MatchmakingAttempt, MatchmakingRecipe, Next, TakeInProgress, Trans,
};
use bloom_offchain::execution_engine::liquidity_book::market_maker::{MakerBehavior, MarketMaker, SpotPrice};
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side};
use bloom_offchain::execution_engine::liquidity_book::stashing_option::StashingOption;
use bloom_offchain::execution_engine::liquidity_book::state::{
    dummy_swap, try_optimized_swap, FillPreview, LiquidityBookSize,
};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use bloom_offchain::execution_engine::liquidity_book::{ExternalLBEvents, LBFeedback, LiquidityBook, TLB};
use either::Either;
use log::trace;
use spectrum_offchain::display::{display_option, display_tuple};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::maker::Maker;
use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::ops::AddAssign;

#[derive(Clone)]
struct FifoState<Taker: Stable, Maker: Stable> {
    queue: VecDeque<Taker::StableId>,
    takers: HashMap<Taker::StableId, Taker>,
    makers: HashMap<Maker::StableId, Maker>,
}

impl<Taker: Stable, Maker: Stable> FifoState<Taker, Maker> {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            takers: HashMap::new(),
            makers: HashMap::new(),
        }
    }

    pub fn queue_size(&self) -> usize {
        self.queue.len()
    }

    pub fn size(&self) -> LiquidityBookSize {
        LiquidityBookSize {
            num_active_takers: self.takers.len(),
            num_active_makers: self.makers.len(),
            num_idle_takers: 0,
            num_idle_makers: 0,
        }
    }

    pub fn append_taker(&mut self, taker: Taker) {
        self.queue.push_back(taker.stable_id());
        self.takers.insert(taker.stable_id(), taker);
    }

    pub fn prepend_taker(&mut self, taker: Taker) {
        self.queue.push_front(taker.stable_id());
        self.takers.insert(taker.stable_id(), taker);
    }

    pub fn remove_taker(&mut self, taker: Taker) {
        self.queue.retain(|id| *id != taker.stable_id());
        self.takers.remove(&taker.stable_id());
    }

    pub fn add_maker(&mut self, maker: Maker) {
        self.makers.insert(maker.stable_id(), maker);
    }

    pub fn remove_maker(&mut self, maker: Maker) {
        self.makers.remove(&maker.stable_id());
    }

    pub fn best_market_maker(&self) -> Option<&Maker>
    where
        Taker: MarketTaker,
        Maker: MarketMaker + Copy,
    {
        self.makers.values().max_by_key(|p| p.quality())
    }

    pub fn preselect_market_maker(
        &self,
        taker: &Taker,
        price: AbsolutePrice,
        demand: u64,
        side: Side,
        optimized: bool,
    ) -> Option<(Maker::StableId, FillPreview)>
    where
        Taker: MarketTaker + TakerBehaviour + Copy,
        Maker: MarketMaker,
    {
        let pools = self
            .makers
            .values()
            .filter(|pool| pool.is_active())
            .filter_map(|p| {
                if optimized {
                    try_optimized_swap(taker, price, demand, side, p)
                        .or_else(|| dummy_swap(taker, demand, side, p))
                } else {
                    dummy_swap(taker, demand, side, p)
                }
            });
        match side {
            Side::Bid => pools.min_by_key(|(_, rp)| rp.price),
            Side::Ask => pools.max_by_key(|(_, rp)| rp.price),
        }
    }

    fn pick_maker_by_id(&mut self, pid: &Maker::StableId) -> Option<Maker> {
        self.makers.remove(pid)
    }

    pub fn pop_taker(&mut self) -> Option<Taker> {
        self.queue.pop_front().and_then(|id| self.takers.remove(&id))
    }
}

#[derive(Clone)]
pub struct Fifo<Taker: Stable, Maker: Stable, Pair, U> {
    state: FifoState<Taker, Maker>,
    backup: Option<FifoState<Taker, Maker>>,
    stash: Vec<Taker>,
    conf: ExecutionConfig<U>,
    pair: Pair,
}

impl<T, M, P, Ctx, U> Maker<P, Ctx> for Fifo<T, M, P, U>
where
    T: Stable,
    M: Stable,
    Ctx: Has<ExecutionConfig<U>>,
{
    fn make(key: P, ctx: &Ctx) -> Self {
        Self::new(ctx.select::<ExecutionConfig<U>>(), key)
    }
}

impl<Taker, Maker, P, U> LBFeedback<Taker, Maker> for Fifo<Taker, Maker, P, U>
where
    Taker: MarketTaker + Stable + Ord + Copy,
    Maker: MarketMaker + Stable + Copy,
{
    fn on_recipe_succeeded(&mut self) {
        self.backup = None;
    }

    fn on_recipe_failed(&mut self) {
        self.rollback(StashingOption::Unstash)
    }
}

impl<Taker: Stable, Maker: Stable, P, U> Fifo<Taker, Maker, P, U> {
    pub fn new(conf: ExecutionConfig<U>, pair: P) -> Self {
        Self {
            state: FifoState::new(),
            backup: None,
            stash: vec![],
            conf,
            pair,
        }
    }

    fn spot_price(&self) -> Option<SpotPrice>
    where
        Taker: MarketTaker,
        Maker: MarketMaker + Copy,
    {
        self.state.best_market_maker().map(|mm| mm.static_price())
    }

    fn backup(&mut self)
    where
        Taker: Clone,
        Maker: Clone,
    {
        self.backup.replace(self.state.clone());
    }

    fn rollback(&mut self, stashing_opt: StashingOption<Taker>)
    where
        Taker: Copy,
    {
        match stashing_opt {
            StashingOption::Stash(mut to_stash) => {
                if let Some(mut backup) = self.backup.take() {
                    for taker in &to_stash {
                        backup.remove_taker(*taker);
                    }
                    self.state = backup;
                    self.stash.append(&mut to_stash);
                };
            }
            StashingOption::Unstash => {
                if let Some(mut backup) = self.backup.take() {
                    for taker in self.stash.drain(..) {
                        backup.append_taker(taker);
                    }
                    self.state = backup;
                };
            }
        }
    }
}

impl<Taker, Maker, P, U> Fifo<Taker, Maker, P, U>
where
    Taker: MarketTaker<U = U> + Stable + Ord + Copy + Display,
    Maker: MarketMaker + Stable + Copy,
    U: PartialOrd,
{
    fn on_take<Any>(&mut self, tx: Next<Taker, Any>) {
        if let Next::Succ(next) = tx {
            self.state.prepend_taker(next);
        }
    }

    fn on_make<Any>(&mut self, tx: Next<Maker, Any>) {
        if let Next::Succ(next) = tx {
            self.state.add_maker(next);
        }
    }
}

impl<Taker, Maker, P, U> LiquidityBook<Taker, Maker, Vec<ExecutionEvent>> for Fifo<Taker, Maker, P, U>
where
    Taker: Stable + MarketTaker<U = U> + TakerBehaviour + Ord + Copy + Display,
    Maker: Stable + MarketMaker<U = U> + MakerBehavior + Copy + Display,
    U: Monoid + AddAssign + PartialOrd + Copy,
    P: Display,
{
    fn attempt(&mut self) -> (Option<MatchmakingRecipe<Taker, Maker>>, Vec<ExecutionEvent>) {
        let mut optimized_matchmaking = true;
        loop {
            trace!(
                "{} Attempting to matchmake (optimized={})",
                self.pair,
                optimized_matchmaking
            );
            let mut batch: MatchmakingAttempt<Taker, Maker, U> = MatchmakingAttempt::empty();
            let mut events = vec![];
            let pre_attempt_size = self.state.size();
            events.push(ExecutionEvent::LiquidityBookSizePreAttempt(pre_attempt_size));
            let mut max_attempts = self.state.queue_size();
            self.backup();
            while batch.execution_units_consumed() < self.conf.execution_cap.soft && batch.num_takes() < 17 {
                if let Some(spot_price) = self.spot_price() {
                    events.push(ExecutionEvent::SpotPrice(spot_price));
                    trace!("{} spot_price: {}", self.pair, spot_price,);
                    if let Some(target_taker) = self.state.pop_taker() {
                        trace!("Selected taker: {}", target_taker);
                        let target_side = target_taker.side();
                        let target_price = target_side.wrap(target_taker.price());
                        let maybe_price_maker = self.state.preselect_market_maker(
                            &target_taker,
                            target_taker.price(),
                            target_taker.input(),
                            target_side,
                            optimized_matchmaking,
                        );
                        trace!(
                            "{} P_target: {}, P_amm: {}",
                            self.pair,
                            target_price.unwrap(),
                            display_option(&maybe_price_maker.map(|(id, fp)| display_tuple((id, fp.price))))
                        );
                        match maybe_price_maker {
                            Some((maker_sid, FillPreview { price, input }))
                                if target_price.overlaps(price) =>
                            {
                                if let Some(maker) = self.state.pick_maker_by_id(&maker_sid) {
                                    trace!("Taker {} matched with {}", target_taker, maker);
                                    let (take, make) =
                                        execute_with_maker(target_taker, maker, target_side.wrap(input));
                                    batch.add_make(make);
                                    batch.add_take(take);
                                    self.on_take(take.result);
                                    self.on_make(make.result);
                                    continue;
                                }
                            }
                            _ => {
                                trace!("Failed to match taker {}", target_taker);
                                self.state.append_taker(target_taker);
                            }
                        }
                    } else {
                        break;
                    }
                } else {
                    events.push(ExecutionEvent::SpotPriceNotAvailable);
                    trace!("{} No liquidity source is available", self.pair);
                }
                if max_attempts > 0 {
                    max_attempts -= 1;
                    continue;
                }
                break;
            }
            trace!("{} Raw batch: {}", self.pair, batch);
            let post_attempt_size = self.state.size();
            events.push(ExecutionEvent::LiquidityBookSizePostAttempt(post_attempt_size));
            match MatchmakingRecipe::try_from(batch, self.conf) {
                Ok(ex_recipe) => {
                    trace!("{} Successfully formed a batch {}", self.pair, ex_recipe);
                    return (Some(ex_recipe), events);
                }
                Err(None) => {
                    trace!("{} Matchmaking attempt failed in fifo", self.pair);
                    self.rollback(StashingOption::Unstash);
                }
                Err(Some(Either::Left(unsatisfied_takers))) => {
                    trace!(
                        "{} Matchmaking attempt failed due to taker limits, retrying",
                        self.pair
                    );
                    self.rollback(StashingOption::Stash(unsatisfied_takers));
                    continue;
                }
                Err(Some(Either::Right(_downgrade_required))) => {
                    trace!(
                        "{} Matchmaking attempt failed due to high execution complexity, retrying",
                        self.pair
                    );
                    self.rollback(StashingOption::Stash(vec![]));
                    optimized_matchmaking = false;
                    continue;
                }
            }
            return (None, events);
        }
    }
}

fn execute_with_maker<Taker, Maker>(
    target_taker: Taker,
    maker: Maker,
    chunk_size: OnSide<u64>,
) -> (TakeInProgress<Taker>, MakeInProgress<Maker>)
where
    Taker: MarketTaker + TakerBehaviour + Copy,
    Maker: MarketMaker + MakerBehavior + Copy,
{
    let next_maker = maker.swap(chunk_size);
    let make = Trans::new(maker, next_maker);
    let trade_output = make.loss().map(|val| val.unwrap()).unwrap_or(0);
    let next_taker = target_taker.with_applied_trade(chunk_size.unwrap(), trade_output);
    let take = Trans::new(target_taker, next_taker);
    (take, make)
}

impl<Taker: Stable, Maker: Stable, Pair, U> ExternalLBEvents<Taker, Maker> for Fifo<Taker, Maker, Pair, U> {
    fn advance_clocks(&mut self, _: u64) {}

    fn update_taker(&mut self, tk: Taker) {
        self.state.append_taker(tk)
    }

    fn remove_taker(&mut self, tk: Taker) {
        self.state.remove_taker(tk)
    }

    fn update_maker(&mut self, mk: Maker) {
        self.state.add_maker(mk)
    }

    fn remove_maker(&mut self, mk: Maker) {
        self.state.remove_maker(mk)
    }
}
