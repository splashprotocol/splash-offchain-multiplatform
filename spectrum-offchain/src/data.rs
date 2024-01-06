use std::fmt::{Debug, Display};
use std::hash::Hash;

use either::Either;
use type_equalities::IsEqual;

use crate::ledger::TryFromLedger;

pub mod order;
pub mod unique_entity;

pub trait Has<T> {
    fn get<U: IsEqual<T>>(&self) -> T;
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

impl<StableId, Version, A, B> EntitySnapshot for Either<A, B>
where
    A: EntitySnapshot<StableId = StableId, Version = Version>,
    B: EntitySnapshot<StableId = StableId, Version = Version>,
    StableId: Copy + Eq + Hash + Debug + Display,
    Version: Copy + Eq + Hash + Display,
{
    type Version = Version;
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

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Debug)]
pub struct Baked<T, V> {
    pub entity: T,
    pub version: V,
}

impl<T, V> Baked<T, V> {
    pub fn new(entity: T, version: V) -> Self {
        Self { entity, version }
    }
}

impl<T, V> Has<V> for Baked<T, V>
where
    V: Copy,
{
    fn get<U: IsEqual<V>>(&self) -> V {
        self.version
    }
}

impl<StableId, Version, T> Stable for Baked<T, Version>
where
    T: Stable<StableId = StableId>,
    StableId: Copy + Eq + Hash + Debug + Display,
    Version: Copy + Eq + Hash + Display,
{
    type StableId = StableId;

    fn stable_id(&self) -> Self::StableId {
        self.entity.stable_id()
    }
}

impl<StableId, Version, T> EntitySnapshot for Baked<T, Version>
where
    T: Stable<StableId = StableId>,
    StableId: Copy + Eq + Hash + Debug + Display,
    Version: Copy + Eq + Hash + Display,
{
    type Version = Version;

    fn version(&self) -> Self::Version {
        self.version
    }
}

impl<T, Version, PairId> Tradable for Baked<T, Version>
where
    PairId: Copy + Eq + Hash + Display,
    T: Tradable<PairId = PairId>,
{
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        self.entity.pair_id()
    }
}

impl<Repr, T, Version> TryFromLedger<Repr, Version> for Baked<T, Version>
where
    T: TryFromLedger<Repr, Version>,
    Version: Copy,
{
    fn try_from_ledger(repr: &Repr, ctx: Version) -> Option<Self> {
        T::try_from_ledger(repr, ctx).map(|r| Baked::new(r, ctx))
    }
}
