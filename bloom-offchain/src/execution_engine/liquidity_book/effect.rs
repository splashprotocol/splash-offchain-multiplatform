use crate::execution_engine::liquidity_book::side::Side;
use crate::execution_engine::SourceId;

#[derive(Debug, Clone)]
pub enum Effect<Fr, Pl> {
    ClocksAdvanced(u64),
    BatchAddFragments(SourceId, Vec<Side<Fr>>),
    BatchRemoveFragments(SourceId),
    PoolUpdated(Pl),
}
