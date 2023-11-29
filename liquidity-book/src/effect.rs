use crate::side::Side;
use crate::types::SourceId;

#[derive(Debug, Clone)]
pub enum Effect<Fr, Pl> {
    ClocksAdvanced(u64),
    BatchAddFragments(SourceId, Vec<Side<Fr>>),
    BatchRemoveFragments(SourceId),
    PoolUpdated(Pl),
}
