use std::cmp::{max, min};

use cml_core::Slot;
use futures::future::Either;

use crate::liquidity_book::effect::Effect;
use crate::liquidity_book::fragment::Fragment;
use crate::liquidity_book::liquidity::fragmented::{FragmentStore, FragmentedLiquidity};
use crate::liquidity_book::liquidity::pooled::{PoolStore, PooledLiquidity};
use crate::liquidity_book::pool::Pool;
use crate::liquidity_book::recipe::{ExecutionRecipe, Fill, PartialFill, Swap, TerminalInstruction};
use crate::liquidity_book::side::{Side, SideMarker};
use crate::liquidity_book::types::ExecutionCost;
use crate::liquidity_book::LiquidityBook;

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

type Recipe = ExecutionRecipe<Fragment<Slot>, Pool>;

impl<FL, PL> LiquidityBook<Slot, Effect<Slot>> for TemporalLiquidityBook<FL, PL>
where
    FL: FragmentedLiquidity<Slot, Fragment<Slot>> + FragmentStore<Slot, Fragment<Slot>>,
    PL: PooledLiquidity<Pool> + PoolStore<Pool>,
{
    fn apply(&mut self, effect: Effect<Slot>) {
        match effect {
            Effect::ClocksAdvanced(new_time) => self.fragmented_liquidity.advance_clocks(new_time),
            Effect::FragmentAdded(fr) => self.fragmented_liquidity.add_fragment(fr),
            Effect::FragmentRemoved(id) => self.fragmented_liquidity.remove_fragment(id),
            Effect::PoolUpdated(new_pool) => self.pooled_liquidity.update_pool(new_pool),
        }
    }
    fn attempt(&mut self) -> Option<ExecutionRecipe<Fragment<Slot>, Pool>> {
        if let Some(best_fr) = self.fragmented_liquidity.pick_either() {
            let mut acc = Recipe::new(best_fr);
            let mut execution_units_left = self.execution_cap.hard;
            loop {
                if let Some(rem) = &acc.remainder {
                    let price_fragments = self.fragmented_liquidity.best_price(!best_fr.marker());
                    let price_in_pools = self.pooled_liquidity.best_price();
                    if price_fragments
                        .iter()
                        .any(|price_in_fragments| price_in_fragments.better_than(price_in_pools))
                        && execution_units_left > self.execution_cap.safe_threshold()
                    {
                        if let Some(opposite_fr) = self.fragmented_liquidity.try_pick(!rem.marker(), |fr| {
                            rem.map(|fr| fr.target.price).overlaps(fr.price)
                                && fr.cost_hint <= execution_units_left
                        }) {
                            execution_units_left -= opposite_fr.cost_hint;
                            match fill_from_fragment(*rem, opposite_fr) {
                                (term_fill_lt, Either::Left(term_fill_rt)) => {
                                    acc.push(TerminalInstruction::Fill(term_fill_lt));
                                    acc.terminate(TerminalInstruction::Fill(term_fill_rt));
                                    break;
                                }
                                (term_fill_lt, Either::Right(partial)) => {
                                    acc.push(TerminalInstruction::Fill(term_fill_lt));
                                    acc.set_remainder(partial);
                                }
                            }
                        }
                    } else {
                        if let Some(pool) = self.pooled_liquidity.try_pick(|pl| {
                            rem.map(|fr| fr.target.price)
                                .overlaps(pl.real_price(rem.marker(), rem.any().remaining_input))
                        }) {
                            let (term_fill, swap) = fill_from_pool(*rem, pool);
                            acc.push(TerminalInstruction::Swap(swap));
                            acc.terminate(TerminalInstruction::Fill(term_fill));
                            break;
                        }
                    }
                }
                break;
            }
            return Some(acc);
        }
        None
    }
}

fn fill_from_fragment<T>(
    target: Side<PartialFill<Fragment<T>>>,
    source: Fragment<T>,
) -> (
    Fill<Fragment<T>>,
    Either<Fill<Fragment<T>>, Side<PartialFill<Fragment<T>>>>,
) {
    match target {
        Side::Bid(mut bid) => {
            let ask = source;
            let price_selector = if bid.target.fee >= ask.fee { min } else { max };
            let price = price_selector(ask.price, bid.target.price);
            let demand_base = ((bid.remaining_input as u128) * price.numer() / price.denom()) as u64;
            let supply_base = ask.input;
            if supply_base > demand_base {
                let quote_input = bid.remaining_input;
                bid.accumulated_output += demand_base;
                (
                    bid.into(),
                    Either::Right(Side::Ask(PartialFill {
                        target: ask,
                        remaining_input: supply_base - demand_base,
                        accumulated_output: quote_input,
                    })),
                )
            } else if supply_base < demand_base {
                let quote_executed = ((supply_base as u128) * price.denom() / price.numer()) as u64;
                bid.remaining_input -= quote_executed;
                bid.accumulated_output += supply_base;
                (Fill::new(ask, quote_executed), Either::Right(Side::Bid(bid)))
            } else {
                let quote_executed = ((supply_base as u128) * price.denom() / price.numer()) as u64;
                bid.accumulated_output += demand_base;
                (bid.into(), Either::Left(Fill::new(ask, quote_executed)))
            }
        }
        Side::Ask(mut ask) => {
            let bid = source;
            let price_selector = if ask.target.fee >= bid.fee { max } else { min };
            let price = price_selector(bid.price, ask.target.price);
            let demand_base = ((bid.input as u128) * price.numer() / price.denom()) as u64;
            let supply_base = ask.remaining_input;
            if supply_base > demand_base {
                ask.remaining_input -= demand_base;
                ask.accumulated_output += bid.input;
                (Fill::new(bid, supply_base), Either::Right(Side::Ask(ask)))
            } else if supply_base < demand_base {
                let quote_executed = ((supply_base as u128) * price.denom() / price.numer()) as u64;
                ask.accumulated_output += quote_executed;
                (
                    ask.into(),
                    Either::Right(Side::Bid(PartialFill {
                        remaining_input: bid.input - quote_executed,
                        target: bid,
                        accumulated_output: supply_base,
                    })),
                )
            } else {
                ask.accumulated_output += bid.input;
                (ask.into(), Either::Left(Fill::new(bid, demand_base)))
            }
        }
    }
}

fn fill_from_pool<T>(
    target: Side<PartialFill<Fragment<T>>>,
    source: Pool,
) -> (Fill<Fragment<T>>, Swap<Pool>) {
    match target {
        Side::Bid(mut bid) => {
            let quote_input = bid.remaining_input;
            let execution_amount = source.output(SideMarker::Bid, quote_input);
            bid.accumulated_output += execution_amount;
            (
                bid.into(),
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
            let execution_amount = source.output(SideMarker::Ask, base_input);
            ask.accumulated_output += execution_amount;
            (
                ask.into(),
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
    #[test]
    fn foo() {}
}
