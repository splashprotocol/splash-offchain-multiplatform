use std::cmp::{max, min};
use std::mem;

use futures::future::Either;

use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use crate::execution_engine::liquidity_book::pool::Pool;
use crate::execution_engine::liquidity_book::recipe::{
    Fill, IntermediateRecipe, PartialFill, Swap, TerminalInstruction,
};
use crate::execution_engine::liquidity_book::side::{Side, SideM};
use crate::execution_engine::liquidity_book::state::{IdleState, TLBState, VersionedState};
use crate::execution_engine::liquidity_book::types::ExecutionCost;

pub mod fragment;
pub mod pool;
pub mod recipe;
pub mod side;
mod state;
pub mod time;
pub mod types;

/// TLB is a Universal Liquidity Aggregator (ULA), it is able to aggregate every piece of composable
/// liquidity available in the market.
///
/// Composable liquidity falls into two essential categories:
/// (1.) Discrete Fragments of liquidity;
/// (2.) Pooled (according to some AMM formula) liquidity;
pub trait TemporalLiquidityBook<Fr, Pl> {
    fn attempt(&mut self) -> Option<IntermediateRecipe<Fr, Pl>>;
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
    fn on_recipe_failed(&mut self);
}

pub struct ExecutionCap {
    pub soft: ExecutionCost,
    pub hard: ExecutionCost,
}

impl ExecutionCap {
    fn safe_threshold(&self) -> ExecutionCost {
        self.hard - self.soft
    }
}

pub struct TLB<Fr, Pl> {
    state: TLBState<Fr, Pl>,
    execution_cap: ExecutionCap,
}

impl<Fr, Pl> TLB<Fr, Pl> {
    pub fn new(time: u64, conf: ExecutionCap) -> Self {
        Self {
            state: TLBState::new(time),
            execution_cap: conf,
        }
    }
}

impl<Fr, Pl> TLB<Fr, Pl>
where
    Fr: Fragment + OrderState + Ord + Copy,
    Pl: Pool + Copy,
{
    fn on_transition(&mut self, tx: StateTrans<Fr>) {
        if let StateTrans::Active(fr) = tx {
            self.state.pre_add_fragment(fr);
        }
    }
}

impl<Fr, Pl> TemporalLiquidityBook<Fr, Pl> for TLB<Fr, Pl>
where
    Fr: Fragment + OrderState + Copy + Ord,
    Pl: Pool + Copy,
{
    fn attempt(&mut self) -> Option<IntermediateRecipe<Fr, Pl>> {
        if let Some(best_fr) = self.state.pick_best_fr_either() {
            let mut recipe = IntermediateRecipe::new(best_fr);
            let mut execution_units_left = self.execution_cap.hard;
            loop {
                if let Some(rem) = &recipe.remainder {
                    let price_fragments = self.state.best_fr_price(!rem.target.side());
                    let price_in_pools = self.state.best_pool_price();
                    match (price_in_pools, price_fragments) {
                        (price_in_pools, Some(price_in_fragments))
                            if price_in_pools
                                .map(|p| price_in_fragments.better_than(p))
                                .unwrap_or(true)
                                && execution_units_left > self.execution_cap.safe_threshold() =>
                        {
                            let rem_side = rem.target.side();
                            if let Some(opposite_fr) = self.state.try_pick_fr(!rem_side, |fr| {
                                rem_side.wrap(rem.target.price()).overlaps(fr.price())
                                    && fr.cost_hint() <= execution_units_left
                            }) {
                                execution_units_left -= opposite_fr.cost_hint();
                                match fill_from_fragment(*rem, opposite_fr) {
                                    FillFromFragment {
                                        term_fill_lt,
                                        fill_rt: Either::Left(term_fill_rt),
                                    } => {
                                        recipe.push(TerminalInstruction::Fill(term_fill_lt));
                                        recipe.terminate(TerminalInstruction::Fill(term_fill_rt));
                                        self.on_transition(term_fill_lt.transition);
                                        self.on_transition(term_fill_rt.transition);
                                    }
                                    FillFromFragment {
                                        term_fill_lt,
                                        fill_rt: Either::Right(partial),
                                    } => {
                                        recipe.push(TerminalInstruction::Fill(term_fill_lt));
                                        recipe.set_remainder(partial);
                                        self.on_transition(term_fill_lt.transition);
                                        continue;
                                    }
                                }
                            }
                        }
                        (Some(_), _) if execution_units_left > 0 => {
                            let rem_side = rem.target.side();
                            if let Some(pool) = self.state.try_pick_pool(|pl| {
                                rem_side
                                    .wrap(rem.target.price())
                                    .overlaps(pl.real_price(rem_side.wrap(rem.remaining_input)))
                            }) {
                                let FillFromPool {
                                    term_fill,
                                    swap: swap,
                                } = fill_from_pool(*rem, pool);
                                recipe.push(TerminalInstruction::Swap(swap));
                                recipe.terminate(TerminalInstruction::Fill(term_fill));
                                self.on_transition(term_fill.transition);
                                self.state.pre_add_pool(swap.transition);
                            }
                        }
                        _ => {}
                    }
                }
                break;
            }
            if recipe.is_complete() {
                return Some(recipe);
            } else {
                self.on_recipe_failed()
            }
        }
        None
    }
}

fn requiring_settled_state<Fr, Pl, F>(book: &mut TLB<Fr, Pl>, f: F)
where
    F: Fn(&mut IdleState<Fr, Pl>),
{
    match book.state {
        TLBState::Idle(ref mut st) => f(st),
        // If there is an attempt to apply external mutations to TLB in a Preview state
        // this is a developer's error so we fail explicitly.
        TLBState::PartialPreview(_) | TLBState::Preview(_) => {
            panic!("PartialPreview|Preview state cannot be externally mutated")
        }
    }
}

impl<Fr, Pl> ExternalTLBEvents<Fr, Pl> for TLB<Fr, Pl>
where
    Fr: Fragment + OrderState + Ord + Copy,
    Pl: Pool + Copy,
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

impl<Fr, Pl> TLBFeedback<Fr, Pl> for TLB<Fr, Pl>
where
    Fr: Fragment + OrderState + Ord + Copy,
    Pl: Pool + Copy,
{
    fn on_recipe_succeeded(&mut self) {
        match &mut self.state {
            TLBState::Idle(_) => {}
            TLBState::PartialPreview(st) => {
                let new_st = st.commit();
                mem::swap(&mut self.state, &mut TLBState::Idle(new_st));
            }
            TLBState::Preview(st) => {
                let new_st = st.commit();
                mem::swap(&mut self.state, &mut TLBState::Idle(new_st));
            }
        }
    }

    fn on_recipe_failed(&mut self) {
        match &mut self.state {
            TLBState::Idle(_) => {}
            TLBState::PartialPreview(st) => {
                let new_st = st.rollback();
                mem::swap(&mut self.state, &mut TLBState::Idle(new_st));
            }
            TLBState::Preview(st) => {
                let new_st = st.rollback();
                mem::swap(&mut self.state, &mut TLBState::Idle(new_st));
            }
        }
    }
}

struct FillFromFragment<Fr> {
    /// [Fill] is always paired with the next state of the underlying order.
    term_fill_lt: Fill<Fr>,
    /// In the case of [PartialFill] calculation of the next state is delayed until matching halts.
    fill_rt: Either<Fill<Fr>, PartialFill<Fr>>,
}

fn fill_from_fragment<Fr>(lhs: PartialFill<Fr>, rhs: Fr) -> FillFromFragment<Fr>
where
    Fr: Fragment + OrderState + Copy,
{
    match lhs.target.side() {
        SideM::Bid => {
            let mut bid = lhs;
            let ask = rhs;
            let price_selector = if bid.target.weight() >= ask.weight() {
                min
            } else {
                max
            };
            let price = price_selector(ask.price(), bid.target.price());
            let demand_base = ((bid.remaining_input as u128) * price.denom() / price.numer()) as u64;
            let supply_base = ask.input();
            if supply_base > demand_base {
                let quote_input = bid.remaining_input;
                bid.accumulated_output += demand_base;
                FillFromFragment {
                    term_fill_lt: bid.filled_unsafe(),
                    fill_rt: Either::Right(PartialFill {
                        target: ask,
                        remaining_input: supply_base - demand_base,
                        accumulated_output: quote_input,
                    }),
                }
            } else if supply_base < demand_base {
                let quote_executed = ((supply_base as u128) * price.denom() / price.numer()) as u64;
                bid.remaining_input -= quote_executed;
                bid.accumulated_output += supply_base;
                FillFromFragment {
                    term_fill_lt: Fill::new(
                        ask,
                        ask.with_updated_liquidity(ask.input(), quote_executed),
                        quote_executed,
                    ),
                    fill_rt: Either::Right(bid),
                }
            } else {
                let quote_executed = ((supply_base as u128) * price.denom() / price.numer()) as u64;
                bid.accumulated_output += demand_base;
                FillFromFragment {
                    term_fill_lt: bid.filled_unsafe(),
                    fill_rt: Either::Left(Fill::new(
                        ask,
                        ask.with_updated_liquidity(ask.input(), quote_executed),
                        quote_executed,
                    )),
                }
            }
        }
        SideM::Ask => {
            let mut ask = lhs;
            let bid = rhs;
            let price_selector = if ask.target.weight() >= bid.weight() {
                max
            } else {
                min
            };
            let price = price_selector(bid.price(), ask.target.price());
            let demand_base = ((bid.input() as u128) * price.denom() / price.numer()) as u64;
            let supply_base = ask.remaining_input;
            if supply_base > demand_base {
                ask.remaining_input -= demand_base;
                ask.accumulated_output += bid.input();
                FillFromFragment {
                    term_fill_lt: Fill::new(
                        bid,
                        bid.with_updated_liquidity(bid.input(), demand_base),
                        demand_base,
                    ),
                    fill_rt: Either::Right(ask),
                }
            } else if supply_base < demand_base {
                let quote_executed = ((supply_base as u128) * price.denom() / price.numer()) as u64;
                ask.accumulated_output += quote_executed;
                FillFromFragment {
                    term_fill_lt: ask.filled_unsafe(),
                    fill_rt: Either::Right(PartialFill {
                        remaining_input: bid.input() - quote_executed,
                        target: bid,
                        accumulated_output: supply_base,
                    }),
                }
            } else {
                ask.accumulated_output += bid.input();
                FillFromFragment {
                    term_fill_lt: ask.filled_unsafe(),
                    fill_rt: Either::Left(Fill::new(
                        bid,
                        bid.with_updated_liquidity(bid.input(), demand_base),
                        demand_base,
                    )),
                }
            }
        }
    }
}

struct FillFromPool<Fr, Pl> {
    term_fill: Fill<Fr>,
    swap: Swap<Pl>,
}

fn fill_from_pool<Fr, Pl>(lhs: PartialFill<Fr>, pool: Pl) -> FillFromPool<Fr, Pl>
where
    Fr: Fragment + OrderState + Copy,
    Pl: Pool + Copy,
{
    match lhs.target.side() {
        SideM::Bid => {
            let mut bid = lhs;
            let quote_input = bid.remaining_input;
            let (execution_amount, next_pool) = pool.swap(Side::Bid(quote_input));
            bid.accumulated_output += execution_amount;
            let swap = Swap {
                target: pool,
                transition: next_pool,
                side: SideM::Bid,
                input: quote_input,
                output: execution_amount,
            };
            FillFromPool {
                term_fill: bid.filled_unsafe(),
                swap,
            }
        }
        SideM::Ask => {
            let mut ask = lhs;
            let base_input = ask.remaining_input;
            let (execution_amount, next_pool) = pool.swap(Side::Ask(base_input));
            ask.accumulated_output += execution_amount;
            let swap = Swap {
                target: pool,
                transition: next_pool,
                side: SideM::Ask,
                input: base_input,
                output: execution_amount,
            };
            FillFromPool {
                term_fill: ask.filled_unsafe(),
                swap,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::future::Either;

    use crate::execution_engine::liquidity_book::fragment::StateTrans;
    use crate::execution_engine::liquidity_book::pool::Pool;
    use crate::execution_engine::liquidity_book::recipe::{
        Fill, IntermediateRecipe, PartialFill, Swap, TerminalInstruction,
    };
    use crate::execution_engine::liquidity_book::side::{Side, SideM};
    use crate::execution_engine::liquidity_book::state::tests::{SimpleCFMMPool, SimpleOrderPF};
    use crate::execution_engine::liquidity_book::time::TimeBounds;
    use crate::execution_engine::liquidity_book::types::BasePrice;
    use crate::execution_engine::liquidity_book::{
        fill_from_fragment, fill_from_pool, ExecutionCap, ExternalTLBEvents, FillFromFragment, FillFromPool,
        TemporalLiquidityBook, TLB,
    };
    use crate::execution_engine::types::StableId;

    #[test]
    fn recipe_fill_fragment_from_fragment() {
        // Assuming pair ADA/USDT @ 0.37
        let o1 = SimpleOrderPF::new(SideM::Ask, 2000, BasePrice::new(36, 100), 1000);
        let o2 = SimpleOrderPF::new(SideM::Bid, 370, BasePrice::new(37, 100), 990);
        let p1 = SimpleCFMMPool {
            pool_id: StableId::random(),
            reserves_base: 1000000000000000,
            reserves_quote: 370000000000000,
            fee_num: 997,
        };
        let mut book = TLB::new(
            0,
            ExecutionCap {
                soft: 10000,
                hard: 16000,
            },
        );
        book.add_fragment(o1);
        book.add_fragment(o2);
        book.update_pool(p1);
        let recipe = book.attempt();
        let p2 = recipe
            .clone()
            .unwrap()
            .terminal
            .iter()
            .find_map(|i| match i {
                TerminalInstruction::Fill(_) => None,
                TerminalInstruction::Swap(swap) => Some(swap.transition),
            })
            .unwrap();
        let expected_recipe = IntermediateRecipe {
            terminal: vec![
                TerminalInstruction::Fill(Fill {
                    target: o2,
                    transition: StateTrans::EOL,
                    removed_input: o2.input,
                    added_output: 1000,
                }),
                TerminalInstruction::Swap(Swap {
                    target: p1,
                    transition: p2,
                    side: SideM::Ask,
                    input: 1000,
                    output: 368,
                }),
                TerminalInstruction::Fill(Fill {
                    target: o1,
                    transition: StateTrans::EOL,
                    removed_input: o1.input,
                    added_output: 738,
                }),
            ],
            remainder: None,
        };
        assert_eq!(recipe, Some(expected_recipe));
    }

    #[test]
    fn fill_fragment_from_fragment() {
        // Assuming pair ADA/USDT @ 0.37
        let fr1 = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Ask,
            input: 1000,
            accumulated_output: 0,
            price: BasePrice::new(37, 100),
            fee: 1000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let fr2 = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Bid,
            input: 370,
            accumulated_output: 0,
            price: BasePrice::new(37, 100),
            fee: 1000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let FillFromFragment {
            term_fill_lt,
            fill_rt: term_fill_rt,
        } = fill_from_fragment(PartialFill::new(fr1), fr2);
        assert_eq!(term_fill_lt.added_output, fr2.input);
        match term_fill_rt {
            Either::Left(fill_rt) => assert_eq!(fill_rt.added_output, fr1.input),
            Either::Right(_) => panic!(),
        }
    }

    #[test]
    fn fill_fragment_from_fragment_partial() {
        // Assuming pair ADA/USDT @ 0.37
        let fr1 = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Ask,
            input: 1000,
            accumulated_output: 0,
            price: BasePrice::new(37, 100),
            fee: 2000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let fr2 = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Bid,
            input: 210,
            accumulated_output: 0,
            price: BasePrice::new(37, 100),
            fee: 2000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let FillFromFragment {
            term_fill_lt,
            fill_rt: term_fill_rt,
        } = fill_from_fragment(PartialFill::new(fr1), fr2);
        assert_eq!(
            term_fill_lt.added_output,
            ((fr2.input as u128) * fr1.price.denom() / fr1.price.numer()) as u64
        );
        match term_fill_rt {
            Either::Right(fill_rt) => assert_eq!(fill_rt.accumulated_output, fr2.input),
            Either::Left(_) => panic!(),
        }
    }

    #[test]
    fn prefer_fragment_with_better_fee() {
        // Assuming pair ADA/USDT @ ask price 0.37, bid price 0.36
        let ask_fr = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Ask,
            input: 1000,
            accumulated_output: 0,
            price: BasePrice::new(36, 100),
            fee: 1000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let bid_fr = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Bid,
            input: 360,
            accumulated_output: 0,
            price: BasePrice::new(37, 100),
            fee: 2000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let FillFromFragment {
            term_fill_lt,
            fill_rt: term_fill_rt,
        } = fill_from_fragment(PartialFill::new(ask_fr), bid_fr);
        assert_eq!(term_fill_lt.added_output, bid_fr.input);
        match term_fill_rt {
            Either::Left(fill_rt) => assert_eq!(fill_rt.added_output, ask_fr.input),
            Either::Right(_) => panic!(),
        }
    }

    #[test]
    fn fill_reminder_from_pool() {
        // Assuming pair ADA/USDT @ ask price 0.360, real price in pool 0.364.
        let ask_fr = SimpleOrderPF {
            source: StableId::random(),
            side: SideM::Ask,
            input: 1000,
            accumulated_output: 0,
            price: BasePrice::new(36, 100),
            fee: 1000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let pf = PartialFill {
            target: ask_fr,
            remaining_input: 500,
            accumulated_output: 180,
        };
        let pool = SimpleCFMMPool {
            pool_id: StableId::random(),
            reserves_base: 100000000000000,
            reserves_quote: 36600000000000,
            fee_num: 997,
        };
        let real_price_in_pool = pool.real_price(Side::Ask(pf.remaining_input));
        let FillFromPool { term_fill, swap } = fill_from_pool(pf, pool);
        assert_eq!(swap.input, pf.remaining_input);
        assert_eq!(
            (term_fill.added_output - pf.accumulated_output) as u128,
            pf.remaining_input as u128 * real_price_in_pool.numer() / real_price_in_pool.denom()
        );
    }
}
