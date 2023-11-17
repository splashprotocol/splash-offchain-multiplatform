use crate::liquidity_book::match_candidates::MatchCandidates;

mod execution_recipe;
mod liquidity;
mod market_effect;
mod match_candidates;
mod temporal_liquidity_book;
mod types;

/// Aggregates composable liquidity.
pub trait LiquidityBook<Eff> {
    fn apply_effect(&mut self, effects: Vec<Eff>);
    fn pull_candidates(&self) -> Option<MatchCandidates>;
}
