use std::fmt::{Debug, Display};
use std::hash::Hash;

use either::Either;
use type_equalities::IsEqual;

use crate::ledger::TryFromLedger;

pub mod order;
pub mod unique_entity;

/// Indicates presence of type [T] in implementor's type.
/// Enables data polymorphism.
pub trait Has<T> {
    /// Use this when there are multiple [Has] bounds on a single type.
    fn get_labeled<U: IsEqual<T>>(&self) -> T;
    /// Use this otherwise.
    fn get(&self) -> T {
        self.get_labeled::<T>()
    }
}

pub trait Stable {
    /// Unique identifier of the underlying entity which persists among different versions.
    type StableId: Copy + Eq + Hash + Debug + Display;
    fn stable_id(&self) -> Self::StableId;
}

pub trait EntitySnapshot: Stable {
    /// Unique version of the [EntitySnapshot].
    type Version: Copy + Eq + Hash + Display;

    fn version(&self) -> Self::Version;
}

pub trait VersionUpdater: EntitySnapshot {
    fn update_version(&mut self, new_version: Self::Version);
}

impl<StableId, A, B> Stable for Either<A, B>
where
    A: Stable<StableId = StableId>,
    B: Stable<StableId = StableId>,
    StableId: Copy + Eq + Hash + Debug + Display,
{
    type StableId = StableId;
    fn stable_id(&self) -> Self::StableId {
        match self {
            Either::Left(a) => a.stable_id(),
            Either::Right(b) => b.stable_id(),
        }
    }
}

impl<StableId, EntityVersion, A, B> EntitySnapshot for Either<A, B>
where
    A: EntitySnapshot<StableId = StableId, Version = EntityVersion>,
    B: EntitySnapshot<StableId = StableId, Version = EntityVersion>,
    StableId: Copy + Eq + Hash + Debug + Display,
    EntityVersion: Copy + Eq + Hash + Display,
{
    type Version = EntityVersion;
    fn version(&self) -> Self::Version {
        match self {
            Either::Left(a) => a.version(),
            Either::Right(b) => b.version(),
        }
    }
}

/// A tradable entity.
pub trait Tradable {
    type PairId: Copy + Eq + Hash + Display;
    fn pair_id(&self) -> Self::PairId;
}

impl<PairId, A, B> Tradable for Either<A, B>
where
    PairId: Copy + Eq + Hash + Display,
    A: Tradable<PairId = PairId>,
    B: Tradable<PairId = PairId>,
{
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        match self {
            Either::Left(x) => x.pair_id(),
            Either::Right(x) => x.pair_id(),
        }
    }
}

/// A baked entity [T] i.e. [T] can no longer be modified.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Debug)]
pub struct Baked<T> {
    pub entity: T,
}

impl<T> Baked<T> {
    pub fn new(entity: T) -> Self {
        Self { entity }
    }
}

impl<StableId, T> Stable for Baked<T>
where
    T: Stable<StableId = StableId>,
    StableId: Copy + Eq + Hash + Debug + Display,
{
    type StableId = StableId;

    fn stable_id(&self) -> Self::StableId {
        self.entity.stable_id()
    }
}

impl<T, PairId> Tradable for Baked<T>
where
    PairId: Copy + Eq + Hash + Display,
    T: Tradable<PairId = PairId>,
{
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        self.entity.pair_id()
    }
}

impl<Repr, T, C> TryFromLedger<Repr, C> for Baked<T>
where
    T: TryFromLedger<Repr, C>,
    C: Copy,
{
    fn try_from_ledger(repr: &Repr, ctx: C) -> Option<Self> {
        T::try_from_ledger(repr, ctx).map(|r| Baked::new(r))
    }
}
