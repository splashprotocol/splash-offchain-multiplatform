use crate::liquidity_book::liquidity::{AnyFragment, AnyPool, OneSideLiquidity};
use crate::liquidity_book::types::SourceId;

#[derive(Debug, Clone)]
pub enum MarketEffect<T> {
    ClocksAdvanced(T),
    FragmentAdded(OneSideLiquidity<AnyFragment<T>>),
    FragmentRemoved(SourceId),
    PoolUpdated(AnyPool),
}
