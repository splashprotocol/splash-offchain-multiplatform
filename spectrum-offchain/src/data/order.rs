use std::hash::Hash;

use type_equalities::IsEqual;

use crate::data::Has;

pub trait UniqueOrder {
    type TOrderId: Eq + Hash;
    fn get_self_ref(&self) -> Self::TOrderId;
}

impl<T> UniqueOrder for T
where
    T: SpecializedOrder,
{
    type TOrderId = <T as SpecializedOrder>::TOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.get_self_ref()
    }
}

impl<T> Has<T::TOrderId> for T
where
    T: UniqueOrder,
{
    fn get_labeled<U: IsEqual<T::TOrderId>>(&self) -> T::TOrderId {
        self.get_self_ref()
    }
}

/// An order specialized for a concrete pool.
pub trait SpecializedOrder {
    type TOrderId: Copy + Eq + Hash;
    type TPoolId: Copy + Eq + Hash;

    fn get_self_ref(&self) -> Self::TOrderId;
    fn get_pool_ref(&self) -> Self::TPoolId;
}

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
pub enum OrderUpdate<TNewOrd, TElimOrd> {
    NewOrder(TNewOrd),
    OrderEliminated(TElimOrd),
}

#[derive(Debug, Clone)]
pub struct OrderLink<TOrd: SpecializedOrder> {
    pub order_id: TOrd::TOrderId,
    pub pool_id: TOrd::TPoolId,
}

impl<TOrd: SpecializedOrder> From<TOrd> for OrderLink<TOrd> {
    fn from(o: TOrd) -> Self {
        Self {
            order_id: o.get_self_ref(),
            pool_id: o.get_pool_ref(),
        }
    }
}

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
pub struct PendingOrder<TOrd> {
    pub order: TOrd,
    pub timestamp: i64,
}

impl<TOrd> From<ProgressingOrder<TOrd>> for PendingOrder<TOrd> {
    fn from(po: ProgressingOrder<TOrd>) -> Self {
        Self {
            order: po.order,
            timestamp: po.timestamp,
        }
    }
}

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
pub struct SuspendedOrder<TOrd> {
    pub order: TOrd,
    pub timestamp: i64,
}

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
pub struct ProgressingOrder<TOrd> {
    pub order: TOrd,
    pub timestamp: i64,
}
