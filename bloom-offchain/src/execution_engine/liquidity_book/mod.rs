use std::fmt::{Debug, Display};
use std::ops::AddAssign;

use algebra_core::monoid::Monoid;
use log::{trace, warn};
use num_rational::Ratio;
use primitive_types::U256;

use crate::display::{display_option, display_tuple};
use crate::execution_engine::liquidity_book::core::{
    MakeInProgress, MatchmakingAttempt, MatchmakingRecipe, Next, TakeInProgress, Trans,
};
use crate::execution_engine::liquidity_book::fragment::{MarketTaker, TakerBehaviour};
use crate::execution_engine::liquidity_book::market_maker::{MakerBehavior, MarketMaker, SpotPrice};
use spectrum_offchain::data::{Has, Stable};
use spectrum_offchain::maker::Maker;

use crate::execution_engine::liquidity_book::side::OnSide::{Ask, Bid};
use crate::execution_engine::liquidity_book::side::{OnSide, Side};
use crate::execution_engine::liquidity_book::stashing_option::StashingOption;
use crate::execution_engine::liquidity_book::state::queries::{max_by_distance_to_spot, max_by_volume};
use crate::execution_engine::liquidity_book::state::{IdleState, TLBState};
use crate::execution_engine::liquidity_book::types::{AbsolutePrice, RelativePrice};
use crate::execution_engine::types::Time;

pub mod core;
pub mod fragment;
pub mod interpreter;
pub mod market_maker;
pub mod side;
pub mod stashing_option;
mod state;
pub mod time;
pub mod types;
pub mod weight;

/// TLB is a Universal Liquidity Aggregator (ULA), it is able to aggregate every piece of composable
/// liquidity available in the market.
///
/// Composable liquidity falls into two essential categories:
/// (1.) Discrete Fragments of liquidity;
/// (2.) Pooled (according to some AMM formula) liquidity;
pub trait TemporalLiquidityBook<Taker, Maker> {
    fn attempt(&mut self) -> Option<MatchmakingRecipe<Taker, Maker>>;
}

/// TLB API for external events affecting its state.
pub trait ExternalTLBEvents<T, M> {
    fn advance_clocks(&mut self, new_time: u64);
    fn add_fragment(&mut self, fr: T);
    fn remove_fragment(&mut self, fr: T);
    fn update_pool(&mut self, pool: M);
    fn remove_pool(&mut self, pool: M);
}

/// TLB API for feedback events affecting its state.
pub trait TLBFeedback<Fr, Pl> {
    fn on_recipe_succeeded(&mut self);
    fn on_recipe_failed(&mut self, stashing_opt: StashingOption<Fr>);
}

#[derive(Debug, Copy, Clone)]
pub struct ExecutionCap<U> {
    pub soft: U,
    pub hard: U,
}

#[derive(Debug, Clone)]
pub struct TLB<Taker, Maker: Stable, U> {
    state: TLBState<Taker, Maker>,
    execution_cap: ExecutionCap<U>,
}

impl<Taker, Maker, U> TLBFeedback<Taker, Maker> for TLB<Taker, Maker, U>
where
    Taker: MarketTaker + Ord + Copy,
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
    Maker: Stable,
{
    pub fn new(time: u64, conf: ExecutionCap<U>) -> Self {
        Self {
            state: TLBState::new(time),
            execution_cap: conf,
        }
    }

    fn spot_price(&self) -> Option<SpotPrice>
    where
        Taker: MarketTaker,
        Maker: MarketMaker + Copy,
    {
        self.state.best_market_maker().map(|mm| mm.static_price())
    }
}

impl<Taker, Maker, U> TLB<Taker, Maker, U>
where
    Taker: MarketTaker<U = U> + Ord + Copy + Display,
    Maker: MarketMaker + Stable + Copy,
    U: PartialOrd,
{
    fn on_take<Any>(&mut self, tx: Next<Taker, Any>) {
        if let Next::Succ(next) = tx {
            self.state.pre_add_taker(next);
        }
    }

    fn on_make<Any>(&mut self, tx: Next<Maker, Any>) {
        if let Next::Succ(next) = tx {
            self.state.pre_add_maker(next);
        }
    }
}

impl<Taker, Maker, U> TemporalLiquidityBook<Taker, Maker> for TLB<Taker, Maker, U>
where
    Taker: Stable + MarketTaker<U = U> + TakerBehaviour + Ord + Copy + Display,
    Maker: Stable + MarketMaker<U = U> + MakerBehavior + Copy + Display,
    U: Monoid + AddAssign + PartialOrd + Copy,
{
    fn attempt(&mut self) -> Option<MatchmakingRecipe<Taker, Maker>> {
        loop {
            trace!("Attempting to matchmake");
            let mut batch: MatchmakingAttempt<Taker, Maker, U> = MatchmakingAttempt::empty();
            while batch.execution_units_consumed() < self.execution_cap.soft {
                let spot_price = self.spot_price();
                let price_range = self.state.allowed_price_range();
                trace!("Spot price is: {}", display_option(spot_price));
                trace!("Price range is: {}", price_range);
                if let Some(target_taker) = self.state.pick_active_taker(|fs| {
                    spot_price
                        .map(|sp| max_by_distance_to_spot(fs, sp, price_range))
                        .unwrap_or_else(|| max_by_volume(fs, price_range))
                }) {
                    trace!("Selected taker is: {}", target_taker);
                    let target_side = target_taker.side();
                    let target_price = target_side.wrap(target_taker.price());
                    let maybe_price_counter_taker = self.state.best_taker_price(!target_side);
                    let chunk_offered = batch.next_offered_chunk(&target_taker);
                    let maybe_price_maker = self.state.preselect_market_maker(chunk_offered);
                    trace!(
                        "P_target: {}, P_counter: {}, P_amm: {}",
                        target_price.unwrap(),
                        display_option(maybe_price_counter_taker),
                        display_option(maybe_price_maker.map(display_tuple))
                    );
                    match (maybe_price_counter_taker, maybe_price_maker) {
                        (Some(price_counter_taker), maybe_price_maker)
                            if target_price.overlaps(price_counter_taker.unwrap())
                                && maybe_price_maker
                                    .map(|(_, p)| price_counter_taker.better_than(p))
                                    .unwrap_or(true) =>
                        {
                            if let Some(counter_taker) = self.state.try_pick_taker(!target_side, ok) {
                                trace!("Taker {} matched with {}", target_taker, counter_taker);
                                let make_match =
                                    |ask: &Taker, bid: &Taker| settle_price(ask, bid, spot_price);
                                let (take_a, take_b) =
                                    execute_with_taker(target_taker, counter_taker, make_match);
                                for take in vec![take_a, take_b] {
                                    batch.add_take(take);
                                    self.on_take(take.result);
                                }
                                continue;
                            }
                        }
                        (_, Some((maker_sid, price_maker))) if target_price.overlaps(price_maker) => {
                            if let Some(maker) = self.state.pick_maker_by_id(&maker_sid) {
                                trace!("Taker {} matched with {}", target_taker, maker);
                                let (take, make) = execute_with_maker(target_taker, maker, chunk_offered);
                                if let Ok(_) = batch.add_make(make) {
                                    batch.add_take(take);
                                    self.on_take(take.result);
                                    self.on_make(make.result);
                                    continue;
                                } else {
                                    warn!("Maker {} caused an opposite swap", maker.stable_id());
                                    self.state.pre_add_maker(maker);
                                    self.state.pre_add_taker(target_taker);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                break;
            }
            match MatchmakingRecipe::try_from(batch) {
                Ok(ex_recipe) => {
                    trace!("Successfully formed a batch {}", ex_recipe);
                    return Some(ex_recipe);
                }
                Err(None) => {
                    trace!("Matchmaking attempt failed");
                    self.on_recipe_failed(StashingOption::Unstash);
                }
                Err(Some(unsatisfied_takers)) => {
                    trace!("Matchmaking attempt failed due to taker limits, retrying");
                    self.on_recipe_failed(StashingOption::Stash(unsatisfied_takers));
                    continue;
                }
            }
            return None;
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

fn execute_with_taker<Taker, F>(
    target_taker: Taker,
    counter_taker: Taker,
    matchmaker: F,
) -> (TakeInProgress<Taker>, TakeInProgress<Taker>)
where
    Taker: MarketTaker + TakerBehaviour + Copy,
    F: FnOnce(&Taker, &Taker) -> AbsolutePrice,
{
    let (ask, bid) = match target_taker.side() {
        Side::Ask => (target_taker, counter_taker),
        Side::Bid => (counter_taker, target_taker),
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
    let next_ask = ask.with_applied_trade(base, quote);
    let next_bid = bid.with_applied_trade(quote, base);
    (Trans::new(ask, next_ask), Trans::new(bid, next_bid))
}

fn ok<T>(_: &T) -> bool {
    true
}

impl<Fr, Pl, Ctx, U> Maker<Ctx> for TLB<Fr, Pl, U>
where
    Pl: Stable,
    Ctx: Has<Time> + Has<ExecutionCap<U>>,
{
    fn make(ctx: &Ctx) -> Self {
        Self::new(ctx.select::<Time>().into(), ctx.select::<ExecutionCap<U>>())
    }
}

fn requiring_settled_state<Fr, Pl, U, F>(book: &mut TLB<Fr, Pl, U>, f: F)
where
    Pl: Stable,
    F: Fn(&mut IdleState<Fr, Pl>),
{
    match book.state {
        TLBState::Idle(ref mut st) => f(st),
        // If there is an attempt to apply external mutations to TLB in a Preview state
        // this is a developer's error, so we fail explicitly.
        TLBState::PartialPreview(_) | TLBState::Preview(_) => {
            panic!("PartialPreview|Preview state cannot be externally mutated")
        }
    }
}

impl<Fr, Pl, U> ExternalTLBEvents<Fr, Pl> for TLB<Fr, Pl, U>
where
    Fr: MarketTaker + TakerBehaviour + Ord + Copy + Display,
    Pl: MarketMaker + Stable + Copy + Display + Debug,
{
    fn advance_clocks(&mut self, new_time: u64) {
        requiring_settled_state(self, |st| st.advance_clocks(new_time))
    }

    fn add_fragment(&mut self, fr: Fr) {
        requiring_settled_state(self, |st| st.add_fragment(fr))
    }

    fn remove_fragment(&mut self, fr: Fr) {
        requiring_settled_state(self, |st| st.remove_fragment(fr))
    }

    fn update_pool(&mut self, pool: Pl) {
        requiring_settled_state(self, |st| st.update_pool(pool))
    }

    fn remove_pool(&mut self, pool: Pl) {
        requiring_settled_state(self, |st| st.remove_pool(pool))
    }
}

const MAX_BIAS_PERCENT: u128 = 3;

//                 P_settled
//                     |
// p: >.... P_x ......(.)...... P_index .... P_y.... >
//           |         |           |          |
//          ask     |bias|<=3%...pivot       bid
/// Settle execution price for two interleaving fragments.
fn settle_price<Fr: MarketTaker>(ask: &Fr, bid: &Fr, index_price: Option<SpotPrice>) -> AbsolutePrice {
    let price_ask = ask.price();
    let price_bid = bid.price();
    let price_ask_rat = price_ask.unwrap();
    let price_bid_rat = price_bid.unwrap();
    let d = price_bid_rat - price_ask_rat;
    let pivotal_price = if let Some(index_price) = index_price {
        clamp(index_price.unwrap(), price_ask_rat, price_bid_rat)
    } else {
        price_ask_rat + d / 2
    };
    let fee_ask = ask.fee() as i128;
    let fee_bid = bid.fee() as i128;
    let bias_percent = if fee_ask < fee_bid {
        (-fee_ask * 100).checked_div(fee_bid).unwrap_or(0)
    } else {
        (fee_bid * 100).checked_div(fee_ask).unwrap_or(0)
    };
    let max_deviation = pivotal_price * Ratio::new(MAX_BIAS_PERCENT, 100);
    let deviation = to_signed(max_deviation) * Ratio::new(bias_percent, 100);
    let corrected_price = to_unsigned(to_signed(pivotal_price) + deviation);
    AbsolutePrice::from(clamp(corrected_price, price_ask_rat, price_bid_rat))
}

fn clamp<I: PartialOrd>(value: I, low: I, high: I) -> I {
    if value >= low && value <= high {
        value
    } else if value < low {
        low
    } else {
        high
    }
}

fn to_signed(r: Ratio<u128>) -> Ratio<i128> {
    Ratio::new(*r.numer() as i128, *r.denom() as i128)
}

fn to_unsigned(r: Ratio<i128>) -> Ratio<u128> {
    Ratio::new(*r.numer() as u128, *r.denom() as u128)
}

pub fn linear_output_relative(input: u64, price: RelativePrice) -> Option<u64> {
    u64::try_from((U256::from(input) * U256::from(*price.numer())).checked_div(U256::from(*price.denom()))?)
        .ok()
}

pub fn linear_output_unsafe(input: u64, price: OnSide<AbsolutePrice>) -> u64 {
    match price {
        Bid(price) => (U256::from(input) * U256::from(*price.denom()) / U256::from(*price.numer())).as_u64(),
        Ask(price) => (U256::from(input) * U256::from(*price.numer()) / U256::from(*price.denom())).as_u64(),
    }
}

#[cfg(test)]
mod tests {
    use crate::execution_engine::liquidity_book::fragment::MarketTaker;
    use crate::execution_engine::liquidity_book::market_maker::MarketMaker;
    use crate::execution_engine::liquidity_book::side::Side::{Ask, Bid};
    use crate::execution_engine::liquidity_book::side::{OnSide, Side};
    use crate::execution_engine::liquidity_book::state::tests::{SimpleCFMMPool, SimpleOrderPF};
    use crate::execution_engine::liquidity_book::time::TimeBounds;
    use crate::execution_engine::liquidity_book::types::AbsolutePrice;
    use crate::execution_engine::liquidity_book::{
        execute_with_maker, execute_with_taker, settle_price, ExecutionCap, ExternalTLBEvents,
        TemporalLiquidityBook, TLB,
    };
    use crate::execution_engine::types::StableId;

    #[test]
    fn recipe_fill_fragment_from_fragment_batch() {
        // Assuming pair ADA/USDT @ 0.37
        let o1 = SimpleOrderPF::make(
            Side::Ask,
            35000000,
            AbsolutePrice::new_unsafe(11989509179467966, 1000000000000000),
            0,
            0,
            5994754,
        );
        let o2 = SimpleOrderPF::make(
            Side::Bid,
            103471165,
            AbsolutePrice::new_unsafe(103471165, 6634631),
            0,
            0,
            6634631,
        );
        let mut book = TLB::<_, SimpleCFMMPool, _>::new(
            0,
            ExecutionCap {
                soft: 1000000,
                hard: 1600000,
            },
        );
        vec![o1, o2].into_iter().for_each(|o| book.add_fragment(o));
        let recipe = book.attempt();
        dbg!(recipe);
    }

    #[test]
    fn recipe_fill_fragment_from_fragment() {
        // Assuming pair ADA/USDT @ 0.37
        let o1 = SimpleOrderPF::new(Ask, 20000, AbsolutePrice::new_unsafe(36, 100), 1000);
        let o2 = SimpleOrderPF::new(Bid, 3700, AbsolutePrice::new_unsafe(37, 100), 990);
        let p1 = SimpleCFMMPool {
            pool_id: StableId::random(),
            reserves_base: 1000000,
            reserves_quote: 370000,
            fee_num: 997,
        };
        let p2 = SimpleCFMMPool {
            pool_id: StableId::random(),
            reserves_base: 1000000,
            reserves_quote: 370000,
            fee_num: 997,
        };
        let mut book = TLB::new(
            0,
            ExecutionCap {
                soft: 1000000,
                hard: 1600000,
            },
        );
        book.add_fragment(o1);
        book.add_fragment(o2);
        book.update_pool(p1);
        book.update_pool(p2);
        let recipe = book.attempt();
        dbg!(recipe);
    }

    #[test]
    fn match_taker_with_taker() {
        // Assuming pair ADA/USDT @ 0.37
        let fr1 = SimpleOrderPF {
            source: StableId::random(),
            side: Side::Ask,
            input: 1000,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: AbsolutePrice::new_unsafe(37, 100),
            fee: 0,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let fr2 = SimpleOrderPF {
            source: StableId::random(),
            side: Side::Bid,
            input: 370,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: AbsolutePrice::new_unsafe(37, 100),
            fee: 0,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let make_match = |x: &SimpleOrderPF, y: &SimpleOrderPF| {
            settle_price(x, y, Some(AbsolutePrice::new_unsafe(37, 100).into()))
        };
        let (t1, t2) = execute_with_taker(fr1, fr2, make_match);
        assert_eq!(t1.added_output(), fr2.input);
        assert_eq!(t2.added_output(), fr1.input);
    }

    #[test]
    fn match_taker_with_taker_partial() {
        // Assuming pair ADA/USDT @ 0.37
        let p = AbsolutePrice::new_unsafe(37, 100);
        let fr1 = SimpleOrderPF {
            source: StableId::random(),
            side: Ask,
            input: 1000,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: p,
            fee: 0,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let fr2 = SimpleOrderPF {
            source: StableId::random(),
            side: Bid,
            input: 210,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: p,
            fee: 0,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let make_match = |x: &SimpleOrderPF, y: &SimpleOrderPF| settle_price(x, y, Some(p.into()));
        let (t1, t2) = execute_with_taker(fr1, fr2, make_match);
        assert_eq!(
            t2.added_output(),
            ((fr2.input as u128) * fr1.price.denom() / fr1.price.numer()) as u64
        );
        assert_eq!(t1.added_output(), fr2.input);
    }

    #[test]
    fn fill_reminder_from_pool() {
        // Assuming pair ADA/USDT @ ask price 0.360, real price in pool 0.364.
        let ask_fr = SimpleOrderPF {
            source: StableId::random(),
            side: Ask,
            input: 1000,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: AbsolutePrice::new_unsafe(36, 100),
            fee: 1000,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let pool = SimpleCFMMPool {
            pool_id: StableId::random(),
            reserves_base: 100000000000000,
            reserves_quote: 36600000000000,
            fee_num: 997,
        };
        let real_price_in_pool = pool.real_price(OnSide::Ask(ask_fr.input()));
        let (t, m) = execute_with_maker(ask_fr, pool, OnSide::Ask(ask_fr.input()));
        assert_eq!(m.gain().unwrap().unwrap(), ask_fr.input());
    }

    #[test]
    fn fill_order_from_pool() {
        // Assuming pair ADA/USDT @ ask price 0.360, real price in pool 0.364.
        let ask_fr = SimpleOrderPF {
            source: StableId::random(),
            side: Bid,
            input: 8851624528,
            accumulated_output: 2512730,
            min_marginal_output: 0,
            price: AbsolutePrice::new_unsafe(8851624528, 2114025439),
            fee: 0,
            ex_budget: 0,
            cost_hint: 0,
            bounds: TimeBounds::None,
        };
        let pool = SimpleCFMMPool {
            pool_id: StableId::random(),
            reserves_base: 4296646506159,
            reserves_quote: 1148842702781,
            fee_num: 997,
        };
        let real_price_in_pool = pool.real_price(OnSide::Ask(ask_fr.input()));
        let (t, m) = execute_with_maker(ask_fr, pool, OnSide::Ask(ask_fr.input()));
        assert_eq!(m.gain().unwrap().unwrap(), t.removed_input());
        assert_eq!(m.loss().unwrap().unwrap(), t.added_output());
    }

    #[test]
    fn match_price_biased_towards_best_fee() {
        let ask_price = AbsolutePrice::new_unsafe(30, 100);
        let bid_price = AbsolutePrice::new_unsafe(50, 100);
        let index_price = AbsolutePrice::new_unsafe(40, 100);
        let ask_fr = SimpleOrderPF {
            source: StableId::random(),
            side: Ask,
            input: 1000,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: ask_price,
            fee: 4000,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let bid_fr = SimpleOrderPF {
            source: StableId::random(),
            side: Bid,
            input: 360,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: bid_price,
            fee: 2000,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let make_match = |x: &SimpleOrderPF, y: &SimpleOrderPF| settle_price(x, y, Some(index_price.into()));
        let final_price = make_match(&ask_fr, &bid_fr);
        assert!(final_price.unwrap() - ask_price.unwrap() > bid_price.unwrap() - final_price.unwrap());
    }

    #[test]
    fn match_price_biased_towards_best_fee_() {
        let ask_price = AbsolutePrice::new_unsafe(30, 100);
        let bid_price = AbsolutePrice::new_unsafe(50, 100);
        let index_price = AbsolutePrice::new_unsafe(51, 100);
        let ask_fr = SimpleOrderPF {
            source: StableId::random(),
            side: Ask,
            input: 1000,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: ask_price,
            fee: 4000,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let bid_fr = SimpleOrderPF {
            source: StableId::random(),
            side: Bid,
            input: 360,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: bid_price,
            fee: 2000,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let make_match = |x: &SimpleOrderPF, y: &SimpleOrderPF| settle_price(x, y, Some(index_price.into()));
        let final_price = make_match(&ask_fr, &bid_fr);
        assert!(final_price.unwrap() - ask_price.unwrap() > bid_price.unwrap() - final_price.unwrap());
    }

    #[test]
    fn match_price_always_stays_within_bounds() {
        let ask_price = AbsolutePrice::new_unsafe(37, 100);
        let bid_price = AbsolutePrice::new_unsafe(37, 100);
        let index_price = AbsolutePrice::new_unsafe(40, 100);
        let ask_fr = SimpleOrderPF {
            source: StableId::random(),
            side: Side::Ask,
            input: 1000,
            min_marginal_output: 0,
            accumulated_output: 0,
            price: ask_price,
            fee: 4000,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let bid_fr = SimpleOrderPF {
            source: StableId::random(),
            side: Side::Bid,
            input: 360,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: bid_price,
            fee: 2000,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let make_match = |x: &SimpleOrderPF, y: &SimpleOrderPF| settle_price(x, y, Some(index_price.into()));
        let final_price = make_match(&ask_fr, &bid_fr);
        assert_eq!(final_price, bid_price)
    }

    #[test]
    fn price_overlap() {
        let rem_side = Bid;
        let rem_price = AbsolutePrice::new_unsafe(12692989795594245882, 12061765702237861555);
        let other_fr_price = AbsolutePrice::new_unsafe(1, 1);
        assert!(rem_side.wrap(rem_price).overlaps(other_fr_price))
    }
}
