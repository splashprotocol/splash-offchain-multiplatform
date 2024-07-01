use std::fmt::{Debug, Display};
use std::ops::{AddAssign, Sub};

use algebra_core::monoid::Monoid;
use log::{trace, warn};
use num_rational::Ratio;
use primitive_types::U256;

use crate::execution_engine::liquidity_book::core::{
    MakeInProgress, MatchmakingAttempt, MatchmakingRecipe, Next, TakeInProgress,
};
use spectrum_offchain::data::{Has, Stable};
use spectrum_offchain::maker::Maker;

use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, TakerBehaviour};
use crate::execution_engine::liquidity_book::market_maker::{MakerBehavior, MarketMaker, SpotPrice};

use crate::execution_engine::liquidity_book::side::Side::{Ask, Bid};
use crate::execution_engine::liquidity_book::side::{Side, SideM};
use crate::execution_engine::liquidity_book::stashing_option::StashingOption;
use crate::execution_engine::liquidity_book::state::queries::{max_by_distance_to_spot, max_by_volume};
use crate::execution_engine::liquidity_book::state::{IdleState, TLBState};
use crate::execution_engine::liquidity_book::types::{AbsolutePrice, RelativePrice};
use crate::execution_engine::types::Time;

pub mod core;
pub mod fragment;
pub mod interpreter;
pub mod market_maker;
pub mod recipe;
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
pub trait TemporalLiquidityBook<Taker: Stable, Maker: Stable> {
    fn attempt(&mut self) -> Option<MatchmakingRecipe<Taker, Maker>>;
}

/// TLB API for external events affecting its state.
pub trait ExternalTLBEvents<Fr, Pl> {
    fn advance_clocks(&mut self, new_time: u64);
    fn add_fragment(&mut self, fr: Fr);
    fn remove_fragment(&mut self, fr: Fr);
    fn update_pool(&mut self, pool: Pl);
    fn remove_pool(&mut self, pool: Pl);
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

impl<U: Sub<Output = U> + Copy> ExecutionCap<U> {
    fn safe_threshold(&self) -> U {
        self.hard - self.soft
    }
}

#[derive(Debug, Clone)]
pub struct TLB<Taker, Maker: Stable, U> {
    state: TLBState<Taker, Maker>,
    execution_cap: ExecutionCap<U>,
}

impl<Taker, Maker, U> TLBFeedback<Taker, Maker> for TLB<Taker, Maker, U>
where
    Taker: Fragment + Ord + Copy,
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
        Taker: Fragment,
        Maker: MarketMaker + Copy,
    {
        self.state.best_market_maker().map(|mm| mm.static_price())
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
    U: Monoid + AddAssign + PartialOrd + Copy,
{
    fn attempt(&mut self) -> Option<MatchmakingRecipe<Taker, Maker>> {
        loop {
            trace!("Attempting to matchmake");
            let mut batch: MatchmakingAttempt<Taker, Maker, U> = MatchmakingAttempt::empty();
            while batch.execution_units_consumed() < self.execution_cap.soft {
                let spot_price = self.spot_price();
                trace!("Spot price is: {:?}", spot_price);
                if let Some(target_taker) = self.state.pick_active_taker(|fs| {
                    spot_price
                        .map(|sp| max_by_distance_to_spot(fs, sp))
                        .unwrap_or(max_by_volume(fs))
                }) {
                    trace!("Selected taker is: {:?}", target_taker);
                    let target_side = target_taker.side();
                    let target_price = target_side.wrap(target_taker.price());
                    let maybe_price_counter_taker = self.state.best_fr_price(!target_side);
                    let chunk_offered = batch.next_offered_chunk(&target_taker);
                    let maybe_price_maker = self.state.preselect_market_maker(chunk_offered);
                    match (maybe_price_counter_taker, maybe_price_maker) {
                        (Some(price_counter_taker), maybe_price_maker)
                            if target_price.overlaps(price_counter_taker.unwrap())
                                && maybe_price_maker
                                    .map(|(_, p)| price_counter_taker.better_than(p))
                                    .unwrap_or(true) =>
                        {
                            if let Some(counter_taker) = self.state.try_pick_fr(!target_side, ok) {
                                trace!("Taker {:?} matched with {:?}", target_taker, counter_taker);
                                let make_match =
                                    |ask: &Taker, bid: &Taker| settle_price(ask, bid, spot_price);
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
                                trace!("Taker {:?} matched with {:?}", target_taker, maker);
                                let (take, make) = execute_with_maker(target_taker, maker, chunk_offered);
                                if let Ok(_) = batch.add_make(make) {
                                    batch.add_take(take);
                                    self.on_take(take.result);
                                    self.on_make(make.result);
                                } else {
                                    warn!("Maker {} caused an opposite swap", maker.stable_id());
                                    self.state.pre_add_pool(maker);
                                    self.state.pre_add_fragment(target_taker);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            match MatchmakingRecipe::try_from(batch) {
                Ok(ex_recipe) => {
                    trace!("Successfully formed a batch {:?}", ex_recipe);
                    return Some(ex_recipe);
                }
                Err(None) => {
                    trace!("Matchmaking attempt failed");
                    self.on_recipe_failed(StashingOption::Unstash);
                }
                Err(Some(unsatisfied_fragments)) => {
                    trace!("Matchmaking attempt failed due to taker limits, retrying");
                    self.on_recipe_failed(StashingOption::Stash(unsatisfied_fragments));
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
    chunk_size: Side<u64>,
) -> (TakeInProgress<Taker>, MakeInProgress<Maker>)
where
    Taker: Fragment + TakerBehaviour + Copy,
    Maker: MarketMaker + MakerBehavior + Copy,
{
    let maker_applied = maker.swap(chunk_size);
    let trade_output = maker_applied.loss().map(|val| val.unwrap()).unwrap_or(0);
    let taker_applied = target_taker.with_applied_trade(chunk_size.unwrap(), trade_output);
    (taker_applied, maker_applied)
}

fn execute_with_taker<Taker, F>(
    target_taker: Taker,
    counter_taker: Taker,
    matchmaker: F,
) -> (TakeInProgress<Taker>, TakeInProgress<Taker>)
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
    Fr: Fragment + OrderState + Ord + Copy + Display,
    Pl: MarketMaker + Stable + Copy,
{
    fn advance_clocks(&mut self, new_time: u64) {
        requiring_settled_state(self, |st| st.advance_clocks(new_time))
    }

    fn add_fragment(&mut self, fr: Fr) {
        trace!(target: "tlb", "TLB::add_fragment({})", fr);
        requiring_settled_state(self, |st| st.add_fragment(fr))
    }

    fn remove_fragment(&mut self, fr: Fr) {
        trace!(target: "tlb", "TLB::remove_fragment({})", fr);
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
fn settle_price<Fr: Fragment>(ask: &Fr, bid: &Fr, index_price: Option<SpotPrice>) -> AbsolutePrice {
    let price_ask = ask.price();
    let price_bid = bid.price();
    let price_ask_rat = price_ask.unwrap();
    let price_bid_rat = price_bid.unwrap();
    let d = price_bid_rat - price_ask_rat;
    let pivotal_price = if let Some(index_price) = index_price {
        truncated(index_price.unwrap(), price_ask_rat, price_bid_rat)
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
    AbsolutePrice::from(truncated(corrected_price, price_ask_rat, price_bid_rat))
}

fn truncated<I: PartialOrd>(value: I, low: I, high: I) -> I {
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
    u64::try_from(U256::from(input) * U256::from(*price.numer()) / U256::from(*price.denom())).ok()
}

fn linear_output_unsafe(input: u64, price: Side<AbsolutePrice>) -> u64 {
    match price {
        Bid(price) => (U256::from(input) * U256::from(*price.denom()) / U256::from(*price.numer())).as_u64(),
        Ask(price) => (U256::from(input) * U256::from(*price.numer()) / U256::from(*price.denom())).as_u64(),
    }
}

#[cfg(test)]
mod tests {
    use crate::execution_engine::liquidity_book::fragment::Fragment;
    use crate::execution_engine::liquidity_book::market_maker::MarketMaker;
    use crate::execution_engine::liquidity_book::side::SideM::Bid;
    use crate::execution_engine::liquidity_book::side::{Side, SideM};
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
            SideM::Ask,
            35000000,
            AbsolutePrice::new(11989509179467966, 1000000000000000),
            0,
            0,
            5994754,
        );
        let o2 = SimpleOrderPF::make(
            SideM::Bid,
            103471165,
            AbsolutePrice::new(103471165, 6634631),
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
        let o1 = SimpleOrderPF::new(SideM::Ask, 2000, AbsolutePrice::new(36, 100), 1000);
        let o2 = SimpleOrderPF::new(SideM::Bid, 370, AbsolutePrice::new(37, 100), 990);
        let p1 = SimpleCFMMPool {
            pool_id: StableId::random(),
            reserves_base: 1000000000000000,
            reserves_quote: 370000000000000,
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
        let recipe = book.attempt();
        // let expected_recipe = IntermediateRecipe {
        //     terminal: vec![
        //         TerminalInstruction::Fill(Fill {
        //             target_fr: o2,
        //             next_fr: StateTrans::EOL,
        //             removed_input: o2.input,
        //             added_output: 1000,
        //             budget_used: 990000,
        //             fee_used: 990,
        //         }),
        //         TerminalInstruction::Swap(Take {
        //             target: p1,
        //             transition: p2,
        //             side: SideM::Ask,
        //             input: 1000,
        //             output: 368,
        //         }),
        //         TerminalInstruction::Fill(Fill {
        //             target_fr: o1,
        //             next_fr: StateTrans::EOL,
        //             removed_input: o1.input,
        //             added_output: 738,
        //             budget_used: 738000,
        //             fee_used: 1000,
        //         }),
        //     ],
        //     remainder: None,
        // };
        dbg!(recipe);
    }

    #[test]
    fn fill_fragment_from_fragment() {
        // Assuming pair ADA/USDT @ 0.37
        let fr1 = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Ask,
            input: 1000,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: AbsolutePrice::new(37, 100),
            fee: 1000,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let fr2 = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Bid,
            input: 370,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: AbsolutePrice::new(37, 100),
            fee: 1000,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let make_match = |x: &SimpleOrderPF, y: &SimpleOrderPF| {
            settle_price(x, y, Some(AbsolutePrice::new(37, 100).into()))
        };
        let (t1, t2) = execute_with_taker(fr1, fr2, make_match);
        assert_eq!(t1.added_output(), fr2.input);
        assert_eq!(t2.added_output(), fr1.input);
    }

    #[test]
    fn fill_fragment_from_fragment_partial() {
        // Assuming pair ADA/USDT @ 0.37
        let p = AbsolutePrice::new(37, 100);
        let fr1 = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Ask,
            input: 1000,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: p,
            fee: 2000,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let fr2 = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Bid,
            input: 210,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: p,
            fee: 2000,
            ex_budget: 0,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let make_match = |x: &SimpleOrderPF, y: &SimpleOrderPF| settle_price(x, y, Some(p.into()));
        let (t1, t2) = execute_with_taker(fr1, fr2, make_match);
        assert_eq!(
            t1.added_output(),
            ((fr2.input as u128) * fr1.price.denom() / fr1.price.numer()) as u64
        );
        assert_eq!(t2.added_output(), fr2.input);
    }

    #[test]
    fn fill_reminder_from_pool() {
        // Assuming pair ADA/USDT @ ask price 0.360, real price in pool 0.364.
        let ask_fr = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Ask,
            input: 1000,
            accumulated_output: 0,
            min_marginal_output: 0,
            price: AbsolutePrice::new(36, 100),
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
        let real_price_in_pool = pool.real_price(Side::Ask(ask_fr.input()));
        let (t, m) = execute_with_maker(ask_fr, pool, Side::Ask(ask_fr.input()));
        assert_eq!(m.gain().unwrap().unwrap(), ask_fr.input());
    }

    #[test]
    fn match_price_biased_towards_best_fee() {
        let ask_price = AbsolutePrice::new(30, 100);
        let bid_price = AbsolutePrice::new(50, 100);
        let index_price = AbsolutePrice::new(40, 100);
        let ask_fr = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Ask,
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
            side: SideM::Bid,
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
        let ask_price = AbsolutePrice::new(30, 100);
        let bid_price = AbsolutePrice::new(50, 100);
        let index_price = AbsolutePrice::new(51, 100);
        let ask_fr = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Ask,
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
            side: SideM::Bid,
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
        let ask_price = AbsolutePrice::new(37, 100);
        let bid_price = AbsolutePrice::new(37, 100);
        let index_price = AbsolutePrice::new(40, 100);
        let ask_fr = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Ask,
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
            side: SideM::Bid,
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
        let rem_price = AbsolutePrice::new(12692989795594245882, 12061765702237861555);
        let other_fr_price = AbsolutePrice::new(1, 1);
        assert!(rem_side.wrap(rem_price).overlaps(other_fr_price))
    }
}
