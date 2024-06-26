use std::fmt::Debug;

use algebra_core::monoid::Monoid;
use spectrum_offchain::data::Stable;

use crate::execution_engine::liquidity_book::core::{Make, TryApply};
use crate::execution_engine::liquidity_book::core::{
    MatchmakingAttempt, MatchmakingRecipe, Next, TakerTrans,
};
use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, TakerBehaviour};
use crate::execution_engine::liquidity_book::market_maker::{MakerBehavior, MarketMaker, SpotPrice};
use crate::execution_engine::liquidity_book::side::Side::{Ask, Bid};
use crate::execution_engine::liquidity_book::side::{Side, SideM};
use crate::execution_engine::liquidity_book::stashing_option::StashingOption;
use crate::execution_engine::liquidity_book::state::{max_by_distance_to_spot, max_by_volume, TLBState};
use crate::execution_engine::liquidity_book::types::AbsolutePrice;
use crate::execution_engine::liquidity_book::{
    linear_output_unsafe, settle_price, ExecutionCap, TLBFeedback,
};

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

impl<Taker, Maker, U> TLB<Taker, Maker, U>
where
    Maker: MarketMaker + Stable,
{
    fn spot_price(&self) -> Option<SpotPrice> {
        None
    }
}

impl<Taker, Maker, U> TLB<Taker, Maker, U>
where
    Taker: Fragment<U = U> + Ord + Copy + Debug,
    Maker: MarketMaker + Stable + Copy,
    U: PartialOrd,
{
    fn on_take<Any>(&mut self, tx: Next<Taker, Any>) {
        if let Next::Succ(next) = tx {
            self.state.pre_add_fragment(next);
        }
    }

    fn on_make<Any>(&mut self, tx: Next<Maker, Any>) {
        if let Next::Succ(next) = tx {
            self.state.pre_add_pool(next);
        }
    }
}

impl<Taker, Maker, U> TemporalLiquidityBook<Taker, Maker> for TLB<Taker, Maker, U>
where
    Taker: Stable + Fragment<U = U> + TakerBehaviour + Ord + Copy + Debug,
    Maker: Stable + MarketMaker<U = U> + MakerBehavior + Copy + Debug,
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
                let chunk_offered = target_side.wrap(0); // todo
                let maybe_price_maker = self.state.preselect_market_maker(chunk_offered);
                match (maybe_price_counter_taker, maybe_price_maker) {
                    (Some(price_counter_taker), maybe_price_maker)
                        if maybe_price_maker
                            .map(|(_, p)| price_counter_taker.better_than(p))
                            .unwrap_or(true) =>
                    {
                        if let Some(counter_taker) = self.state.try_pick_fr(!target_side, ok) {
                            //fill target_taker <- counter_taker
                            let make_match = |ask: &Taker, bid: &Taker| settle_price(ask, bid, spot_price);
                            let (take_a, take_b) =
                                execute_with_taker(target_taker, counter_taker, make_match);
                            for take in vec![take_a, take_b] {
                                batch.add_take(take);
                                self.on_take(take.result);
                            }
                        }
                    }
                    (_, Some((maker_sid, price_maker))) if target_price.overlaps(price_maker) => {
                        if let Some(maker) = self.state.take_pool(&maker_sid) {
                            //fill target_taker <- maker
                            let (take, make) = execute_with_maker(target_taker, maker, chunk_offered);
                            batch.add_take(take);
                            batch.add_make(make);
                            self.on_take(take.result);
                            self.on_make(make.result);
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }
}

fn execute_with_maker<Taker, Maker>(
    target_taker: Taker,
    maker: Maker,
    chunk_size: Side<u64>,
) -> (TakerTrans<Taker>, TryApply<Make, Maker>)
where
    Taker: Fragment + TakerBehaviour + Copy,
    Maker: MakerBehavior + Copy,
{
    let maker_applied = maker.swap(chunk_size);
    let taker_applied = target_taker.with_applied_trade(chunk_size.unwrap(), maker_applied.action.output);
    (taker_applied, maker_applied)
}

fn execute_with_taker<Taker, F>(
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
