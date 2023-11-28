use crate::recipe::ExecutionRecipe;

pub mod effect;
pub mod fragment;
pub mod liquidity;
pub mod pool;
pub mod recipe;
pub mod side;
pub mod temporal;
pub mod time;
pub mod types;

/// Aggregates composable liquidity.
pub trait LiquidityBook<Fr, Pl, Eff> {
    fn apply(&mut self, effect: Eff);
    fn attempt(&mut self) -> Option<ExecutionRecipe<Fr, Pl>>;
}
