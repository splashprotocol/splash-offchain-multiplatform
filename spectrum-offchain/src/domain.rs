use std::fmt::{Debug, Display, Formatter};
use std::hash::Hash;

use either::Either;
use serde::de::DeserializeOwned;
use serde::Serialize;
use type_equalities::IsEqual;

use crate::ledger::TryFromLedger;

pub mod event;
pub mod order;

/// Indicates that [T] is a member of some type.
/// Enables data polymorphism.
pub trait Has<T> {
    /// Use this when there are multiple [Has] bounds on a single type.
    fn select<U: IsEqual<T>>(&self) -> T;
    /// Use this otherwise.
    fn get(&self) -> T {
        self.select::<T>()
    }
}

pub trait Identifier: Copy + Clone + Eq + PartialEq {
    type For;
}

pub trait Stable {
    /// Unique identifier of the underlying entity which persists among different versions.
    type StableId: Copy + Eq + Hash + Send + Sync + Display;
    fn stable_id(&self) -> Self::StableId;
    /// Some entities are more stable than others. This flag marks these.
    fn is_quasi_permanent(&self) -> bool;
}

/// One state in a sequence states of an entity.
///
/// Types implementing this trait must also implement the `Stable` trait,
/// ensuring they have a stable identifier and can persist across versions.
pub trait SeqState: Stable {
    /// Whether the state is initial in the sequence.
    fn is_initial(&self) -> bool;
}

pub trait EntitySnapshot: Stable {
    /// Unique version of the [EntitySnapshot].
    type Version: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned;

    fn version(&self) -> Self::Version;
}

impl<StableId, A, B> Stable for Either<A, B>
where
    A: Stable<StableId = StableId>,
    B: Stable<StableId = StableId>,
    StableId: Copy + Eq + Hash + Send + Sync + Debug + Display,
{
    type StableId = StableId;
    fn stable_id(&self) -> Self::StableId {
        match self {
            Either::Left(a) => a.stable_id(),
            Either::Right(b) => b.stable_id(),
        }
    }
    fn is_quasi_permanent(&self) -> bool {
        match self {
            Either::Left(a) => a.is_quasi_permanent(),
            Either::Right(b) => b.is_quasi_permanent(),
        }
    }
}

impl<StableId, A, B> SeqState for Either<A, B>
where
    A: SeqState<StableId = StableId>,
    B: SeqState<StableId = StableId>,
    StableId: Copy + Eq + Hash + Send + Sync + Debug + Display,
{
    fn is_initial(&self) -> bool {
        match self {
            Either::Left(a) => a.is_initial(),
            Either::Right(b) => b.is_initial(),
        }
    }
}

impl<StableId, EntityVersion, A, B> EntitySnapshot for Either<A, B>
where
    A: EntitySnapshot<StableId = StableId, Version = EntityVersion>,
    B: EntitySnapshot<StableId = StableId, Version = EntityVersion>,
    StableId: Copy + Eq + Hash + Send + Sync + Debug + Display,
    EntityVersion: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned,
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

/// A baked entity [T] paired with a computed version [V],
/// i.e. [T] can no longer be modified.
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

impl<T: Display, V: Display> Display for Baked<T, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("Baked({}, version: {})", self.entity, self.version).as_str())
    }
}

impl<T, V> Has<V> for Baked<T, V>
where
    V: Copy,
{
    fn select<U: IsEqual<V>>(&self) -> V {
        self.version
    }
}

impl<StableId, Version, T> Stable for Baked<T, Version>
where
    T: Stable<StableId = StableId>,
    StableId: Copy + Eq + Hash + Send + Sync + Debug + Display,
    Version: Copy + Eq + Hash + Send + Sync + Display,
{
    type StableId = StableId;

    fn stable_id(&self) -> Self::StableId {
        self.entity.stable_id()
    }
    fn is_quasi_permanent(&self) -> bool {
        self.entity.is_quasi_permanent()
    }
}

impl<StableId, Version, T> SeqState for Baked<T, Version>
where
    T: SeqState<StableId = StableId>,
    StableId: Copy + Eq + Hash + Send + Sync + Debug + Display,
    Version: Copy + Eq + Hash + Send + Sync + Display,
{
    fn is_initial(&self) -> bool {
        self.entity.is_initial()
    }
}

impl<StableId, BakedVersion, T> EntitySnapshot for Baked<T, BakedVersion>
where
    T: Stable<StableId = StableId>,
    StableId: Copy + Eq + Hash + Send + Sync + Debug + Display,
    BakedVersion: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned,
{
    type Version = BakedVersion;

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

impl<Repr, T, C, Version> TryFromLedger<Repr, C> for Baked<T, Version>
where
    T: TryFromLedger<Repr, C>,
    Version: Copy,
    C: Clone + Has<Version>,
{
    fn try_from_ledger(repr: &Repr, ctx: &C) -> Option<Self> {
        T::try_from_ledger(repr, ctx).map(|r| Baked::new(r, ctx.select::<Version>()))
    }
}
