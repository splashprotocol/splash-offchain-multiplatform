use crate::execution_engine::liquidity_book::recipe::ExecutionRecipe;

pub mod effect;
pub mod fragment;
pub mod liquidity;
pub mod pool;
pub mod recipe;
pub mod side;
pub mod temporal;
pub mod time;
pub mod types;

/// Universal liquidity aggregator (ULA) - aggregates any piece of composable liquidity available in the market.
/// Composable liquidity falls into two essential categories:
/// (1.) Discrete Fragments of liquidity;
/// (2.) Pooled (according to some AMM formula)  liquidity;
pub trait LiquidityBook<Fr, Pl, Eff> {
    fn apply(&mut self, effect: Eff);
    fn attempt(&mut self) -> Option<ExecutionRecipe<Fr, Pl>>;
}
