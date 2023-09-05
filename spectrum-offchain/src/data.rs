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
    T: OnChainOrder,
{
    type TOrderId = <T as OnChainOrder>::TOrderId;
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

pub trait OnChainOrder {
    type TOrderId: Eq + Hash;
    type TEntityId: Eq + Hash;

    fn get_self_ref(&self) -> Self::TOrderId;
    fn get_entity_ref(&self) -> Self::TEntityId;
}

pub trait OnChainEntity {
    type TEntityId: Eq + Hash;
    type TStateId: Eq + Hash;

    fn get_self_ref(&self) -> Self::TEntityId;

    fn get_self_state_ref(&self) -> Self::TStateId;
}
