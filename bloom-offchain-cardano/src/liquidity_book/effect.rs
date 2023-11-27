use crate::liquidity_book::fragment::Fragment;
use crate::liquidity_book::pool::Pool;
use crate::liquidity_book::side::Side;
use crate::liquidity_book::types::SourceId;

#[derive(Debug, Clone)]
pub enum Effect<T> {
    ClocksAdvanced(T),
    BatchAddFragments(SourceId, Vec<Side<Fragment<T>>>),
    BatchRemoveFragments(SourceId),
    PoolUpdated(SourceId, Pool),
}
