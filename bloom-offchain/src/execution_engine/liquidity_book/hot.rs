use crate::display::display_option;
use crate::execution_engine::liquidity_book::config::{ExecutionCap, ExecutionConfig};
use crate::execution_engine::liquidity_book::core::{MatchmakingAttempt, MatchmakingRecipe, Next};
use crate::execution_engine::liquidity_book::market_maker::{AvailableLiquidity, MakerBehavior, MarketMaker};
use crate::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use crate::execution_engine::liquidity_book::side::Side;
use crate::execution_engine::liquidity_book::state::queries::{max_by_distance_to_spot, max_by_volume};
use crate::execution_engine::liquidity_book::types::AbsolutePrice;
use crate::execution_engine::liquidity_book::{
    execute_with_maker, ExternalLBEvents, LBFeedback, LiquidityBook, TLB,
};
use crate::execution_engine::types::Time;
use algebra_core::monoid::Monoid;
use log::{info, trace, warn};
use spectrum_offchain::data::{Has, Stable};
use spectrum_offchain::maker::Maker;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt::Display;
use std::hash::Hash;
use std::mem;
use std::ops::AddAssign;

#[derive(Clone, Debug)]
struct StateView<T: Stable, M: Stable> {
    takers: HashMap<T::StableId, T>,
    takers_queue: VecDeque<T::StableId>,
    makers: HashMap<M::StableId, M>,
}

impl<T: Stable, M: Stable> StateView<T, M> {
    fn new() -> Self {
        Self {
            takers: HashMap::new(),
            takers_queue: VecDeque::new(),
            makers: HashMap::new(),
        }
    }
    fn enqueue_taker(&mut self, taker: T) {
        let taker_sid = taker.stable_id();
        self.takers_queue.push_back(taker_sid);
        self.takers.insert(taker_sid, taker);
    }
    fn return_taker(&mut self, taker: T) {
        let taker_sid = taker.stable_id();
        self.takers_queue.push_front(taker_sid);
        self.takers.insert(taker_sid, taker);
    }
    fn remove_taker(&mut self, taker: T) {
        self.takers.remove(&taker.stable_id());
    }
    fn update_maker(&mut self, maker: M)
    where
        M: Copy + Display,
    {
        let old = self.makers.insert(maker.stable_id(), maker);
        trace!("Updating maker {} -> {}", display_option(&old), maker);
    }
    fn remove_maker(&mut self, maker: M) {
        self.makers.remove(&maker.stable_id());
    }
}

#[derive(Clone)]
struct IdleState<T: Stable, M: Stable>(StateView<T, M>);

#[derive(Clone)]
struct PreviewState<T: Stable, M: Stable>(/*preview*/ StateView<T, M>, /*backup*/ StateView<T, M>);

impl<T: Stable, M: Stable> PreviewState<T, M> {
    fn move_from(idle_state: &mut IdleState<T, M>) -> Self
    where
        T: Clone,
        M: Clone,
    {
        let mut fresh_view = StateView::new();
        mem::swap(&mut idle_state.0, &mut fresh_view);
        Self(fresh_view.clone(), fresh_view)
    }
}

#[derive(Clone)]
enum State<T: Stable, M: Stable> {
    Idle(IdleState<T, M>),
    Preview(PreviewState<T, M>),
}

impl<T: Stable, M: Stable> State<T, M> {
    fn new() -> Self {
        State::Idle(IdleState(StateView::new()))
    }

    fn preview(&mut self) -> Option<&mut PreviewState<T, M>>
    where
        T: Clone,
        M: Clone,
    {
        match self {
            State::Idle(idle) => {
                // Idle => Preview
                let preview_state = PreviewState::move_from(idle);
                // Preview => self
                mem::swap(self, &mut Self::Preview(preview_state));
                match self {
                    State::Preview(preview) => Some(preview),
                    State::Idle(_) => unreachable!(),
                }
            }
            State::Preview(_) => None,
        }
    }

    fn commit(&mut self) {
        match self {
            State::Idle(_) => (),
            State::Preview(in_progress) => {
                trace!("Preview => Idle");
                let mut preview = StateView::new();
                // InProgress.preview => preview
                mem::swap(&mut in_progress.0, &mut preview);
                // preview => self.state.Idle.view
                mem::swap(&mut State::Idle(IdleState(preview)), self);
            }
        }
    }

    fn rollback(&mut self) {
        match self {
            State::Idle(_) => (),
            State::Preview(in_progress) => {
                trace!("Preview => Idle");
                let mut backup_view = StateView::new();
                // InProgress.backup => backup_view
                mem::swap(&mut in_progress.1, &mut backup_view);
                // backup_view => self.state.Idle.view
                mem::swap(&mut State::Idle(IdleState(backup_view)), self);
            }
        }
    }
}

impl<T: Stable, M: Stable> PreviewState<T, M> {
    fn take_best_maker(&mut self) -> Option<M>
    where
        M: MarketMaker,
    {
        if let Some(sid) = self
            .0
            .makers
            .iter()
            .max_by_key(|(_, m)| m.quality())
            .map(|(sid, _)| *sid)
        {
            return self.0.makers.remove(&sid);
        }
        None
    }

    fn return_maker(&mut self, maker: M)
    where
        M: Copy + Display,
    {
        self.0.update_maker(maker);
    }

    fn on_make<Any>(&mut self, tx: Next<M, Any>)
    where
        M: Copy + Display,
    {
        if let Next::Succ(next) = tx {
            self.0.update_maker(next);
        }
    }

    fn pop_taker(&mut self) -> Option<T> {
        loop {
            if let Some(taker_id) = self.0.takers_queue.pop_front() {
                if let Some(taker) = self.0.takers.remove(&taker_id) {
                    return Some(taker);
                }
            } else {
                break;
            }
        }
        None
    }

    fn return_taker(&mut self, taker: T) {
        self.0.return_taker(taker);
    }

    fn enqueue_taker(&mut self, taker: T) {
        self.0.enqueue_taker(taker);
    }

    fn on_take<Any>(&mut self, tx: Next<T, Any>) {
        if let Next::Succ(next) = tx {
            self.0.enqueue_taker(next);
        }
    }
}

#[derive(Clone)]
pub struct HotLB<T: Stable, M: Stable, U> {
    state: State<T, M>,
    execution_cap: ExecutionCap<U>,
}

impl<T: Stable, M: Stable, U> HotLB<T, M, U> {
    fn new(execution_cap: ExecutionCap<U>) -> Self {
        Self {
            state: State::new(),
            execution_cap,
        }
    }
}

impl<T, M, Ctx, U> Maker<Ctx> for HotLB<T, M, U>
where
    T: Stable,
    M: Stable,
    Ctx: Has<ExecutionConfig<U>>,
{
    fn make(ctx: &Ctx) -> Self {
        Self::new(ctx.select::<ExecutionConfig<U>>().execution_cap)
    }
}

impl<T: Stable, M: Stable + Copy + Display, U> ExternalLBEvents<T, M> for HotLB<T, M, U> {
    fn advance_clocks(&mut self, _: u64) {}

    fn update_taker(&mut self, taker: T) {
        match &mut self.state {
            State::Idle(IdleState(ref mut idle)) => idle.enqueue_taker(taker),
            State::Preview(_) => panic!("Attempt to update InProgress state externally"),
        }
    }

    fn remove_taker(&mut self, taker: T) {
        match &mut self.state {
            State::Idle(IdleState(ref mut idle)) => idle.remove_taker(taker),
            State::Preview(_) => panic!("Attempt to update InProgress state externally"),
        }
    }

    fn update_maker(&mut self, maker: M) {
        match &mut self.state {
            State::Idle(IdleState(ref mut idle)) => idle.update_maker(maker),
            State::Preview(_) => panic!("Attempt to update InProgress state externally"),
        }
    }

    fn remove_maker(&mut self, maker: M) {
        match &mut self.state {
            State::Idle(IdleState(ref mut idle)) => idle.remove_maker(maker),
            State::Preview(_) => panic!("Attempt to update InProgress state externally"),
        }
    }
}

impl<T: Stable, M: Stable, U> LBFeedback<T, M> for HotLB<T, M, U> {
    fn on_recipe_succeeded(&mut self) {
        self.state.commit()
    }

    fn on_recipe_failed(&mut self) {
        self.state.rollback()
    }
}

impl<T, M, U> LiquidityBook<T, M> for HotLB<T, M, U>
where
    T: Stable + MarketTaker<U = U> + TakerBehaviour + Copy + Display,
    M: Stable + MarketMaker<U = U> + MakerBehavior + Copy + Display,
    U: Monoid + AddAssign + PartialOrd + Copy,
{
    fn attempt(&mut self) -> Option<MatchmakingRecipe<T, M>> {
        loop {
            trace!("Attempting to matchmake");
            let mut batch: MatchmakingAttempt<T, M, U> = MatchmakingAttempt::empty();
            if let Some(preview_state) = self.state.preview() {
                while batch.execution_units_consumed() < self.execution_cap.soft {
                    if let Some(taker) = preview_state.pop_taker() {
                        if let Some(maker) = preview_state.take_best_maker() {
                            let side = taker.side();
                            let target_price = side.wrap(taker.price());
                            if let Some(AvailableLiquidity { input, output }) =
                                maker.estimated_trade(side.wrap(taker.input()))
                            {
                                let offered_price = match side {
                                    Side::Bid => AbsolutePrice::new(input, output)?,
                                    Side::Ask => AbsolutePrice::new(output, input)?,
                                };
                                if target_price.overlaps(offered_price) {
                                    trace!("Taker {} matched with {}", taker, maker);
                                    let (take, make) = execute_with_maker(taker, maker, side.wrap(input));
                                    batch.add_make(make);
                                    batch.add_take(take);
                                    preview_state.on_take(take.result);
                                    preview_state.on_make(make.result);
                                    continue;
                                } else {
                                    info!("Taker {} returned to the end of the queue", taker.stable_id());
                                    preview_state.enqueue_taker(taker);
                                }
                            }
                        } else {
                            warn!("No MM found!");
                            preview_state.return_taker(taker);
                        }
                    }
                    break;
                }
            }
            trace!("Raw batch: {}", batch);
            match MatchmakingRecipe::try_from(batch) {
                Ok(ex_recipe) => {
                    trace!("Successfully formed a batch {}", ex_recipe);
                    return Some(ex_recipe);
                }
                Err(None) => {
                    trace!("Matchmaking attempt failed");
                    self.state.rollback();
                }
                Err(Some(_)) => {
                    trace!("Matchmaking attempt failed due to taker limits, retrying");
                    self.state.rollback();
                    continue;
                }
            }
            break;
        }
        None
    }
}
