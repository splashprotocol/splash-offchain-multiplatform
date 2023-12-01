use std::cmp::{max, min};

use futures::future::Either;

use crate::execution_engine::liquidity_book::effect::Effect;
use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use crate::execution_engine::liquidity_book::liquidity::fragmented::{FragmentedLiquidity, FragmentStore};
use crate::execution_engine::liquidity_book::liquidity::pooled::{PooledLiquidity, PoolStore};
use crate::execution_engine::liquidity_book::LiquidityBook;
use crate::execution_engine::liquidity_book::pool::Pool;
use crate::execution_engine::liquidity_book::recipe::{
    ExecutionRecipe, Fill, PartialFill, Swap, TerminalInstruction,
};
use crate::execution_engine::liquidity_book::side::{Side, SideMarker};
use crate::execution_engine::liquidity_book::types::ExecutionCost;

pub struct ExecutionCap {
    pub soft: ExecutionCost,
    pub hard: ExecutionCost,
}

impl ExecutionCap {
    fn safe_threshold(&self) -> ExecutionCost {
        self.hard - self.soft
    }
}

pub struct TemporalLiquidityBook<FL, PL> {
    fragmented_liquidity: FL,
    pooled_liquidity: PL,
    execution_cap: ExecutionCap,
}

impl<Fr, Pl, FL, PL> LiquidityBook<Fr, Pl, Effect<Fr, Pl>> for TemporalLiquidityBook<FL, PL>
where
    Fr: Fragment + Copy,
    Pl: Pool + Copy,
    FL: FragmentedLiquidity<Fr> + FragmentStore<Fr>,
    PL: PooledLiquidity<Pl> + PoolStore<Pl>,
{
    fn apply(&mut self, effect: Effect<Fr, Pl>) {
    }

    fn attempt(&mut self) -> Option<ExecutionRecipe<Fr, Pl>> {
        // if let Some(best_fr) = self.fragmented_liquidity.pick_either() {
        //     let mut acc = ExecutionRecipe::new(best_fr);
        //     let mut execution_units_left = self.execution_cap.hard;
        //     loop {
        //         if let Some(rem) = &acc.remainder {
        //             let price_fragments = self.fragmented_liquidity.best_price(!best_fr.marker());
        //             let price_in_pools = self.pooled_liquidity.best_price();
        //             match (price_in_pools, price_fragments) {
        //                 (price_in_pools, Some(price_in_fragments))
        //                     if price_in_pools
        //                         .map(|p| price_in_fragments.better_than(p))
        //                         .unwrap_or(true)
        //                         && execution_units_left > self.execution_cap.safe_threshold() =>
        //                 {
        //                     if let Some(opposite_fr) =
        //                         self.fragmented_liquidity.try_pick(!rem.marker(), |fr| {
        //                             rem.map(|fr| fr.target.price()).overlaps(fr.price())
        //                                 && fr.cost_hint() <= execution_units_left
        //                         })
        //                     {
        //                         execution_units_left -= opposite_fr.cost_hint();
        //                         match fill_from_fragment(*rem, opposite_fr) {
        //                             FillFromFragment { term_fill_lt: (term_fill_lt, succ_lt), term_fill_rt: Either::Left((term_fill_rt, succ_rt)) } => {
        //                                 acc.push(TerminalInstruction::Fill(term_fill_lt));
        //                                 acc.terminate(TerminalInstruction::Fill(term_fill_rt));
        //                             }
        //                             FillFromFragment { term_fill_lt: (term_fill_lt, succ_lt), term_fill_rt: Either::Right(partial) } => {
        //                                 acc.push(TerminalInstruction::Fill(term_fill_lt));
        //                                 acc.set_remainder(partial);
        //                                 continue;
        //                             }
        //                         }
        //                     }
        //                 }
        //                 (Some(_), _) if execution_units_left > 0 => {
        //                     if let Some(pool) = self.pooled_liquidity.try_pick(|pl| {
        //                         rem.map(|fr| fr.target.price())
        //                             .overlaps(pl.real_price(rem.map(|fr| fr.remaining_input)))
        //                     }) {
        //                         let (term_fill, swap) = fill_from_pool(*rem, pool);
        //                         acc.push(TerminalInstruction::Swap(swap));
        //                         acc.terminate(TerminalInstruction::Fill(term_fill));
        //                     }
        //                 }
        //                 _ => {}
        //             }
        //         }
        //         break;
        //     }
        //     if acc.is_complete() {
        //         return Some(acc);
        //     } else {
        //         // return liquidity if recipe failed.
        //         for fr in acc.disassemble() {
        //             match fr {
        //                 Either::Left(fr) => self.fragmented_liquidity.return_fr(fr),
        //                 Either::Right(pl) => self.pooled_liquidity.update_pool(pl),
        //             }
        //         }
        //     }
        // }
        None
    }
}

struct FillFromFragment<Fr> {
    /// [Fill] is always paired with the next state of the underlying order.
    term_fill_lt: (Fill<Fr>, StateTrans<Fr>),
    /// In the case of [PartialFill] calculation of the next state is delayed until matching halts.
    term_fill_rt: Either<(Fill<Fr>, StateTrans<Fr>), PartialFill<Fr>>,
}

fn fill_from_fragment<Fr>(
    lhs: PartialFill<Fr>,
    rhs: Fr,
) -> FillFromFragment<Fr>
where
    Fr: Fragment + OrderState + Copy,
{
    match lhs.target.side() {
        SideMarker::Bid => {
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
                    term_fill_lt: bid.into_filled(),
                    term_fill_rt: Either::Right(PartialFill {
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
                    term_fill_lt: (Fill::new(ask, quote_executed), ask.with_updated_liquidity(ask.input(), quote_executed)),
                    term_fill_rt: Either::Right(bid),
                }
            } else {
                let quote_executed = ((supply_base as u128) * price.denom() / price.numer()) as u64;
                bid.accumulated_output += demand_base;
                FillFromFragment {
                    term_fill_lt: bid.into_filled(),
                    term_fill_rt: Either::Left((Fill::new(ask, quote_executed), ask.with_updated_liquidity(ask.input(), quote_executed))),
                }
            }
        }
        SideMarker::Ask => {
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
                    term_fill_lt: (Fill::new(bid, demand_base), bid.with_updated_liquidity(bid.input(), demand_base)),
                    term_fill_rt:  Either::Right(ask),
                }
            } else if supply_base < demand_base {
                let quote_executed = ((supply_base as u128) * price.denom() / price.numer()) as u64;
                ask.accumulated_output += quote_executed;
                FillFromFragment {
                    term_fill_lt: ask.into_filled(),
                    term_fill_rt: Either::Right(PartialFill {
                        remaining_input: bid.input() - quote_executed,
                        target: bid,
                        accumulated_output: supply_base,
                    }),
                }
            } else {
                ask.accumulated_output += bid.input();
                FillFromFragment {
                    term_fill_lt: ask.into_filled(),
                    term_fill_rt: Either::Left((Fill::new(bid, demand_base), bid.with_updated_liquidity(bid.input(), demand_base))),
                }
            }
        }
    }
}

fn fill_from_pool<Fr, Pl>(target: Side<PartialFill<Fr>>, source: Pl) -> (Side<Fill<Fr>>, Swap<Pl>)
where
    Pl: Pool,
{
    match target {
        Side::Bid(mut bid) => {
            let quote_input = bid.remaining_input;
            let execution_amount = source.output(Side::Bid(quote_input));
            bid.accumulated_output += execution_amount;
            (
                Side::Bid(bid.into()),
                Swap {
                    target: source,
                    side: SideMarker::Bid,
                    input: quote_input,
                    output: execution_amount,
                },
            )
        }
        Side::Ask(mut ask) => {
            let base_input = ask.remaining_input;
            let execution_amount = source.output(Side::Ask(base_input));
            ask.accumulated_output += execution_amount;
            (
                Side::Ask(ask.into()),
                Swap {
                    target: source,
                    side: SideMarker::Ask,
                    input: base_input,
                    output: execution_amount,
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use cml_core::Slot;
    use futures::future::Either;
    use num_rational::Ratio;

    use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
    use crate::execution_engine::liquidity_book::pool::Pool;
    use crate::execution_engine::liquidity_book::recipe::PartialFill;
    use crate::execution_engine::liquidity_book::side::{Side, SideMarker};
    use crate::execution_engine::liquidity_book::temporal::{fill_from_fragment, fill_from_pool, FillFromFragment};
    use crate::execution_engine::liquidity_book::time::TimeBounds;
    use crate::execution_engine::liquidity_book::types::{ExecutionCost, Price};
    use crate::execution_engine::SourceId;

    #[test]
    fn fill_fragment_from_fragment() {
        // Assuming pair ADA/USDT @ 0.37
        let fr1 = SimpleOrder {
            source: SourceId::random(),
            side: SideMarker::Ask,
            input: 1000,
            accumulated_output: 0,
            price: Ratio::new(37, 100),
            fee: 1000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let fr2 = SimpleOrder {
            source: SourceId::random(),
            side: SideMarker::Bid,
            input: 370,
            accumulated_output: 0,
            price: Ratio::new(37, 100),
            fee: 1000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let FillFromFragment { term_fill_lt, term_fill_rt } = fill_from_fragment(PartialFill::new(fr1), fr2);
        assert_eq!(term_fill_lt.0.output, fr2.input);
        match term_fill_rt {
            Either::Left(fill_rt) => assert_eq!(fill_rt.0.output, fr1.input),
            Either::Right(_) => panic!(),
        }
    }

    #[test]
    fn fill_fragment_from_fragment_partial() {
        // Assuming pair ADA/USDT @ 0.37
        let fr1 = SimpleOrder {
            source: SourceId::random(),
            side: SideMarker::Ask,
            input: 1000,
            accumulated_output: 0,
            price: Ratio::new(37, 100),
            fee: 2000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let fr2 = SimpleOrder {
            source: SourceId::random(),
            side: SideMarker::Bid,
            input: 210,
            accumulated_output: 0,
            price: Ratio::new(37, 100),
            fee: 2000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let FillFromFragment { term_fill_lt, term_fill_rt } = fill_from_fragment(PartialFill::new(fr1), fr2);
        assert_eq!(
            term_fill_lt.0.output,
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
        let ask_fr = SimpleOrder {
            source: SourceId::random(),
            side: SideMarker::Ask,
            input: 1000,
            accumulated_output: 0,
            price: Ratio::new(36, 100),
            fee: 1000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let bid_fr = SimpleOrder {
            source: SourceId::random(),
            side: SideMarker::Bid,
            input: 360,
            accumulated_output: 0,
            price: Ratio::new(37, 100),
            fee: 2000,
            cost_hint: 100,
            bounds: TimeBounds::None,
        };
        let FillFromFragment { term_fill_lt, term_fill_rt } = fill_from_fragment(PartialFill::new(ask_fr), bid_fr);
        assert_eq!(term_fill_lt.0.output, bid_fr.input);
        match term_fill_rt {
            Either::Left(fill_rt) => assert_eq!(fill_rt.0.output, ask_fr.input),
            Either::Right(_) => panic!(),
        }
    }

    #[test]
    fn fill_reminder_from_pool() {
        // Assuming pair ADA/USDT @ ask price 0.360, real price in pool 0.364.
        let ask_fr = SimpleOrder {
            source: SourceId::random(),
            side: SideMarker::Ask,
            input: 1000,
            accumulated_output: 0,
            price: Ratio::new(36, 100),
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
            reserves_base: 100000000000000,
            reserves_quote: 36600000000000,
            fee_num: 997,
        };
        println!("Static price in pool: {}", pool.static_price());
        let real_price_in_pool = pool.real_price(Side::Ask(pf.remaining_input));
        println!("Real price in pool: {}", real_price_in_pool);
        let (fill_lt, swap) = fill_from_pool(Side::Ask(pf), pool);
        assert_eq!(swap.input, pf.remaining_input);
        assert_eq!(
            (fill_lt.any().output - pf.accumulated_output) as u128,
            pf.remaining_input as u128 * real_price_in_pool.numer() / real_price_in_pool.denom()
        );
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct SimpleOrder {
        pub source: SourceId,
        pub side: SideMarker,
        pub input: u64,
        pub accumulated_output: u64,
        pub price: Price,
        pub fee: u64,
        pub cost_hint: ExecutionCost,
        pub bounds: TimeBounds<Slot>,
    }

    impl Fragment for SimpleOrder {
        fn source(&self) -> SourceId {
            self.source
        }

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

    impl OrderState for SimpleOrder {
        fn with_updated_time(self, time: u64) -> StateTrans<Self> {
            StateTrans::Active(self)
        }

        fn with_updated_liquidity(mut self, removed_input: u64, added_output: u64) -> StateTrans<Self> {
            self.input -= removed_input;
            self.accumulated_output += added_output;
            if self.input > 0 {  StateTrans::Active(self) } else {StateTrans::EOL(self)}
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct SimpleCFMMPool {
        reserves_base: u64,
        reserves_quote: u64,
        fee_num: u64,
    }

    impl Pool for SimpleCFMMPool {
        fn static_price(&self) -> Price {
            Ratio::new(self.reserves_quote as u128, self.reserves_base as u128)
        }

        fn real_price(&self, input: Side<u64>) -> Price {
            match input {
                Side::Bid(quote_input) => {
                    let base_output = self.output(Side::Bid(quote_input));
                    Ratio::new(quote_input as u128, base_output as u128)
                }
                Side::Ask(base_input) => {
                    let quote_output = self.output(Side::Ask(base_input));
                    Ratio::new(quote_output as u128, base_input as u128)
                }
            }
        }

        fn output(&self, input: Side<u64>) -> u64 {
            match input {
                Side::Bid(quote_input) => {
                    ((self.reserves_base as u128) * (quote_input as u128) * (self.fee_num as u128)
                        / ((self.reserves_quote as u128) * 1000u128
                            + (quote_input as u128) * (self.fee_num as u128))) as u64
                }
                Side::Ask(base_input) => {
                    ((self.reserves_quote as u128) * (base_input as u128) * (self.fee_num as u128)
                        / ((self.reserves_base as u128) * 1000u128
                            + (base_input as u128) * (self.fee_num as u128))) as u64
                }
            }
        }
    }
}
