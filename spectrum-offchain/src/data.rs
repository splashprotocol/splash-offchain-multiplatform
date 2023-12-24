use std::fmt::Display;
use std::hash::Hash;

use futures::future::Either;
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

pub trait EntitySnapshot {
    /// Unique identifier of the underlying entity which persists among different versions.
    type StableId: Copy + Eq + Hash + Display;
    /// Unique version of the [EntitySnapshot].
    type Version: Copy + Eq + Hash + Display;

    fn stable_id(&self) -> Self::StableId;

    fn version(&self) -> Self::Version;
}

/// A tradable entity.
pub trait Tradable {
    type PairId: Copy + Eq + Hash + Display;
    fn pair_id(&self) -> Self::PairId;
}

impl<StableId, Version, A, B> EntitySnapshot for Either<A, B>
where
    A: EntitySnapshot<StableId = StableId, Version = Version>,
    B: EntitySnapshot<StableId = StableId, Version = Version>,
    StableId: Copy + Eq + Hash + Display,
    Version: Copy + Eq + Hash + Display,
{
    type StableId = StableId;
    type Version = Version;
    fn stable_id(&self) -> Self::StableId {
        match self {
            Either::Left(a) => a.stable_id(),
            Either::Right(b) => b.stable_id(),
        }
    }
    fn version(&self) -> Self::Version {
        match self {
            Either::Left(a) => a.version(),
            Either::Right(b) => b.version(),
        }
    }
}
