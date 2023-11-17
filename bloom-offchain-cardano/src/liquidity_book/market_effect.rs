use crate::liquidity_book::liquidity::{LiquidityFragment, PooledLiquidity};
use crate::liquidity_book::types::SourceId;

#[derive(Debug, Copy, Clone)]
pub enum MarketEffect<T> {
    ClocksAdvanced(T),
    FragmentAdded(LiquidityFragment<T>),
    PoolUpdated(PooledLiquidity),
    SourceRemoved(SourceId),
}
