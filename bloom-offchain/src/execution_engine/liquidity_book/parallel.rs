use std::cmp::min;
use std::fmt::Debug;

use algebra_core::monoid::Monoid;
use spectrum_offchain::data::Stable;

use crate::execution_engine::liquidity_book::core::{
    Make, MatchmakingAttempt, MatchmakingRecipe, MatchmakingStep, TakerTrans, TryApply,
};
use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, TakerBehaviour};
use crate::execution_engine::liquidity_book::market_maker::{MarketMaker, SpotPrice};
use crate::execution_engine::liquidity_book::side::Side::{Ask, Bid};
use crate::execution_engine::liquidity_book::side::{Side, SideM};
use crate::execution_engine::liquidity_book::stashing_option::StashingOption;
use crate::execution_engine::liquidity_book::state::{max_by_distance_to_spot, max_by_volume, TLBState};
use crate::execution_engine::liquidity_book::types::{AbsolutePrice, InputAsset};
use crate::execution_engine::liquidity_book::{linear_output_unsafe, ExecutionCap, TLBFeedback};

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
    step: MatchmakingStep,
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

impl<Taker, Maker, U> TLB<Taker, Maker, U>
where
    Maker: MarketMaker + Stable,
{
    fn spot_price(&self) -> Option<SpotPrice> {
        None
    }
}

impl<Taker, Maker, U> TemporalLiquidityBook<Taker, Maker> for TLB<Taker, Maker, U>
where
    Taker: Stable + Fragment<U = U> + OrderState + Ord + Copy + Debug,
    Maker: Stable + MarketMaker<U = U> + Copy + Debug,
    U: Monoid + PartialOrd + Copy,
{
    fn attempt(&mut self) -> Option<MatchmakingRecipe<Taker, Maker>> {
        let mut batch: MatchmakingAttempt<Taker, Maker, U> = MatchmakingAttempt::empty();
        while batch.execution_units_consumed() < self.execution_cap.soft {
            let spot_price = self.spot_price();
            if let Some(target_taker) = self.state.pick_taker(|fs| {
                spot_price
                    .map(|sp| max_by_distance_to_spot(fs, sp))
                    .unwrap_or(max_by_volume(fs))
            }) {
                let target_side = target_taker.side();
                let target_price = target_side.wrap(target_taker.price());
                let maybe_price_counter_taker = self.state.best_fr_price(!target_side);
                let chunk_offered = target_side.wrap(0); //take.next_chunk_offered(self.step);
                let maybe_price_maker = self.state.preselect_market_maker(chunk_offered);
                match (maybe_price_counter_taker, maybe_price_maker) {
                    (Some(price_counter_taker), maybe_price_maker)
                        if maybe_price_maker
                            .map(|(_, p)| price_counter_taker.better_than(p))
                            .unwrap_or(true) =>
                    {
                        if let Some(counter_taker) = self.state.try_pick_fr(!target_side, ok) {
                            //fill target_taker <- counter_taker
                        }
                    }
                    (_, Some((maker_sid, price_maker))) if target_price.overlaps(price_maker) => {
                        if let Some(maker) = self.state.take_pool(&maker_sid) {
                            //fill target_taker <- maker
                        }
                    }
                    _ => {}
                }
            }
        }
        None

        // loop {
        //     while batch.execution_units_consumed() < self.execution_cap.soft {
        //         if let Some(best_taker) = self.state.try_pick_fr(self.attempt_side, ok) {
        //             let take = TakeInProgress::new(best_taker);
        //             loop {
        //                 if batch.execution_units_consumed() < self.execution_cap.soft {
        //                     // 1. Take a chunk of remaining input from remainder
        //                     // 2. Take liquidity from best counter-offer (takers/makers)
        //                     let rem_side = take.target.side();
        //                     let rem_price = rem_side.wrap(take.target.price());
        //                     let maybe_price_counter_taker = self.state.best_fr_price(!rem_side);
        //                     let chunk_offered = take.next_chunk_offered(self.step);
        //                     let maybe_price_maker = self.state.preselect_market_maker(chunk_offered);
        //                     match (maybe_price_counter_taker, maybe_price_maker) {
        //                         (Some(price_counter_taker), maybe_price_maker)
        //                             if maybe_price_maker
        //                                 .map(|(_, p)| price_counter_taker.better_than(p))
        //                                 .unwrap_or(true) => {}
        //                         (_, Some((maker_sid, price_maker))) if rem_price.overlaps(price_maker) => {}
        //                         _ => {}
        //                     }
        //                 }
        //                 break;
        //             }
        //         }
        //         break;
        //     }
        // }
    }
}

fn fill_from_maker<Taker, Maker>(
    target_taker: Taker,
    maker: Maker,
) -> (TakerTrans<Taker>, TryApply<Make, Maker>) {
    todo!()
}

fn fill_from_taker<Taker, F>(
    target_taker: Taker,
    counter_taker: Taker,
    matchmaker: F,
) -> (TakerTrans<Taker>, TakerTrans<Taker>)
where
    Taker: Fragment + TakerBehaviour + Copy,
    F: FnOnce(&Taker, &Taker) -> AbsolutePrice,
{
    let (ask, bid) = match target_taker.side() {
        SideM::Bid => (counter_taker, target_taker),
        SideM::Ask => (target_taker, counter_taker),
    };
    let price = matchmaker(&ask, &bid);
    let quote_input = bid.input();
    let demand_base = linear_output_unsafe(quote_input, Bid(price));
    let supply_base = ask.input();
    let (quote, base) = if supply_base > demand_base {
        (quote_input, demand_base)
    } else if supply_base < demand_base {
        let quote_executed = linear_output_unsafe(supply_base, Ask(price));
        (quote_executed, supply_base)
    } else {
        (quote_input, demand_base)
    };
    let next_bid = bid.with_applied_trade(quote, base);
    let next_ask = ask.with_applied_trade(base, quote);
    (next_bid, next_ask)
}

fn ok<'a, T>(_: &'a T) -> bool {
    true
}
