use std::fmt::Display;
use std::hash::Hash;

use type_equalities::IsEqual;

pub mod order;
pub mod unique_entity;

pub trait Has<T> {
    fn get<U: IsEqual<T>>(&self) -> T;
}

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
    fn get<U: IsEqual<T::TOrderId>>(&self) -> T::TOrderId {
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

pub trait OnChainEntity {
    type TEntityId: Copy + Eq + Hash;
    type TStateId: Copy + Eq + Hash;

    fn get_self_ref(&self) -> Self::TEntityId;

    fn get_self_state_ref(&self) -> Self::TStateId;
}
