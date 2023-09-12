use crate::data::PoolId;

pub struct Base;

pub struct Quote;

pub struct PoolNft;

pub struct ClassicalOrder<Id, Ord> {
    pub id: Id,
    pub pool_id: PoolId,
    pub order: Ord,
}

pub enum ClassicalOrderAction {
    Apply,
    Refund,
}
