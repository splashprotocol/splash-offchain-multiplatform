use std::fmt::Display;
use std::hash::Hash;
use std::marker::PhantomData;

use spectrum_offchain::data::{EntitySnapshot, Has, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;

/// Entity bundled with its source.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Bundled<T, Source, Version> {
    pub entity: T,
    pub source: Source,
    version: PhantomData<Version>,
}

impl<T, Source, Version> Bundled<T, Source, Version> {
    pub fn new(entity: T, source: Source) -> Self {
        Self {
            entity,
            source,
            version: PhantomData,
        }
    }
}

impl<T, Source, Version> Stable for Bundled<T, Source, Version>
where
    T: Stable,
{
    type StableId = T::StableId;
    fn stable_id(&self) -> Self::StableId {
        self.entity.stable_id()
    }
}

impl<T, O, V> EntitySnapshot for Bundled<T, O, V>
where
    T: Stable,
    O: Has<V>,
    V: Eq + Copy + Display + Hash,
{
    type Version = V;
    fn version(&self) -> Self::Version {
        self.source.get()
    }
}

impl<T, Source, V> Tradable for Bundled<T, Source, V>
where
    T: Tradable,
{
    type PairId = T::PairId;
    fn pair_id(&self) -> Self::PairId {
        self.entity.pair_id()
    }
}

impl<T, Source, Ctx, Version> TryFromLedger<Source, Ctx> for Bundled<T, Source, Version>
where
    T: TryFromLedger<Source, Ctx>,
    Source: Clone,
{
    fn try_from_ledger(repr: &Source, ctx: Ctx) -> Option<Self> {
        T::try_from_ledger(&repr, ctx).map(|res| Bundled::new(res, repr.clone()))
    }
}
