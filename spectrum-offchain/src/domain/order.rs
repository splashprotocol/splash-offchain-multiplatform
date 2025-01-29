use std::hash::Hash;

use either::Either;
use serde::{Deserialize, Serialize};

pub trait UniqueOrder {
    type TOrderId: Copy + Clone + Eq + Hash;
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

/// An order specialized for a concrete pool.
pub trait SpecializedOrder {
    type TOrderId: Copy + Eq + Hash + Send + Sync;
    type TPoolId: Copy + Eq + Hash + Send + Sync;

    fn get_self_ref(&self) -> Self::TOrderId;
    fn get_pool_ref(&self) -> Self::TPoolId;
}

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
pub enum OrderUpdate<TNewOrd, TElimOrd> {
    Created(TNewOrd),
    Eliminated(TElimOrd),
}

impl<TNewOrd, TElimOrd> From<Either<TElimOrd, TNewOrd>> for OrderUpdate<TNewOrd, TElimOrd> {
    fn from(value: Either<TElimOrd, TNewOrd>) -> Self {
        match value {
            Either::Left(consumed) => OrderUpdate::Eliminated(consumed),
            Either::Right(produced) => OrderUpdate::Created(produced),
        }
    }
}

#[derive(Debug, Copy, Clone)]
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

#[derive(Debug, Hash, Clone, Eq, PartialEq, Deserialize, Serialize)]
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
