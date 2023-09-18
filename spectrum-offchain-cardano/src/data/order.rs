use spectrum_offchain::data::UniqueOrder;

use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::{OnChainOrderId, PoolId};

pub struct Base;

pub struct Quote;

pub struct PoolNft;

#[derive(Debug, Clone)]
pub struct ClassicalOrder<Id, Ord> {
    pub id: Id,
    pub pool_id: PoolId,
    pub order: Ord,
}

pub enum ClassicalOrderAction {
    Apply,
    Refund,
}

#[derive(Debug, Clone)]
pub enum ClassicalOnChainOrder {
    Swap(ClassicalOnChainLimitSwap),
}

impl UniqueOrder for ClassicalOnChainOrder {
    type TOrderId = OnChainOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        match self {
            ClassicalOnChainOrder::Swap(swap) => swap.id,
        }
    }
}
