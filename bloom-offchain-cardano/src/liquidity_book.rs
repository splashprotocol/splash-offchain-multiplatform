use crate::liquidity_book::fragment::Fragment;
use crate::liquidity_book::pool::Pool;
use crate::liquidity_book::recipe::ExecutionRecipe;

mod effect;
mod fragment;
mod liquidity;
mod pool;
mod recipe;
mod side;
mod temporal;
mod types;

/// Aggregates composable liquidity.
pub trait LiquidityBook<T, Eff> {
    fn apply(&mut self, effect: Eff);
    fn attempt(&mut self) -> Option<ExecutionRecipe<Fragment<T>, Pool>>;
}
