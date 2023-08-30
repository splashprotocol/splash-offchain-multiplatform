use std::hash::Hash;

use type_equalities::IsEqual;

pub mod order;
pub mod unique_entity;

pub trait Has<T> {
    fn get<U: IsEqual<T>>(&self) -> T;
}

pub trait OnChainOrder {
    type TOrderId: Eq + Hash;
    type TEntityId: Eq + Hash;

    fn get_self_ref(&self) -> Self::TOrderId;
    fn get_entity_ref(&self) -> Self::TEntityId;
}

impl<T> Has<T::TOrderId> for T
where
    T: OnChainOrder,
{
    fn get<U: IsEqual<T::TOrderId>>(&self) -> T::TOrderId {
        self.get_self_ref()
    }
}

pub trait OnChainEntity {
    type TEntityId: Eq + Hash;
    type TStateId: Eq + Hash;

    fn get_self_ref(&self) -> Self::TEntityId;

    fn get_self_state_ref(&self) -> Self::TStateId;
}
