use std::fmt::Debug;
use std::mem;

use log::trace;

use algebra_core::monoid::Monoid;
use spectrum_offchain::data::Stable;

use crate::execution_engine::liquidity_book::core::{MatchmakingAttempt, MatchmakingRecipe, TakeInProgress};
use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState};
use crate::execution_engine::liquidity_book::market_maker::MarketMaker;
use crate::execution_engine::liquidity_book::side::SideM;
use crate::execution_engine::liquidity_book::stashing_option::StashingOption;
use crate::execution_engine::liquidity_book::state::TLBState;
use crate::execution_engine::liquidity_book::{ExecutionCap, TLBFeedback};

/// TLB is a Universal Liquidity Aggregator (ULA), it is able to aggregate every piece of composable
/// liquidity available in the market.
///
/// Composable liquidity falls into two essential categories:
/// (1.) Discrete Fragments of liquidity;
/// (2.) Pooled (according to some AMM formula) liquidity;
pub trait TemporalLiquidityBook<Taker: Stable, Maker: Stable> {
    fn attempt(&mut self) -> Option<MatchmakingRecipe<Taker, Maker>>;
}

#[derive(Debug, Clone)]
pub struct TLB<Taker, Maker: Stable, U> {
    state: TLBState<Taker, Maker>,
    execution_cap: ExecutionCap<U>,
    attempt_side: SideM,
}

impl<Taker, Maker, U> TLBFeedback<Taker, Maker> for TLB<Taker, Maker, U>
where
    Taker: Fragment + OrderState + Ord + Copy,
    Maker: MarketMaker + Stable + Copy,
{
    fn on_recipe_succeeded(&mut self) {
        self.state.commit();
    }

    fn on_recipe_failed(&mut self, stashing_opt: StashingOption<Taker>) {
        self.state.rollback(stashing_opt);
    }
}

impl<Taker, Maker, U> TemporalLiquidityBook<Taker, Maker> for TLB<Taker, Maker, U>
where
    Taker: Stable + Fragment<U = U> + OrderState + Ord + Copy + Debug,
    Maker: Stable + MarketMaker<U = U> + Copy + Debug,
    U: Monoid + PartialOrd + Copy,
{
    fn attempt(&mut self) -> Option<MatchmakingRecipe<Taker, Maker>> {
        loop {
            let mut batch: MatchmakingAttempt<_, _, U> = MatchmakingAttempt::empty();
            let mut both_sides_tried = false;
            while batch.execution_units_consumed() < self.execution_cap.soft {
                if let Some(best_taker) = self.state.try_pick_fr(self.attempt_side, ok) {
                    batch.set_remainder(TakeInProgress::new(best_taker));
                    loop {
                        if let Some(rem) = batch.remainder() {
                            if batch.execution_units_consumed() < self.execution_cap.soft {
                                // 1. Take a chunk of remaining input from remainder
                                // 2. Take liquidity from best counter-offer (takers/makers)
                                
                            }
                        }
                        break;
                    }
                }
                self.attempt_side = !self.attempt_side;
                break;
            }
            match MatchmakingRecipe::try_from(batch) {
                Ok(ex_recipe) => return Some(ex_recipe),
                Err(None) => {
                    self.on_recipe_failed(StashingOption::Unstash);
                    if mem::replace(&mut both_sides_tried, true) {
                        trace!("Trying to matchmake on the other side: {}", self.attempt_side);
                        continue;
                    }
                }
                Err(Some(unsatisfied_fragments)) => {
                    self.on_recipe_failed(StashingOption::Stash(unsatisfied_fragments));
                    continue;
                }
            }
            return None;
        }
    }
}

fn ok<'a, T>(_: &'a T) -> bool {
    true
}
