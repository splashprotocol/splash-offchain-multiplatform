use spectrum_offchain::data::{EntitySnapshot, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;

/// Entity bundled with its source.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Bundled<T, Source>(pub T, pub Source);

impl<T, Source> Stable for Bundled<T, Source>
where
    T: Stable,
{
    type StableId = T::StableId;
    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
}

impl<T, Source> EntitySnapshot for Bundled<T, Source>
where
    T: EntitySnapshot,
{
    type Version = T::Version;
    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

impl<T, Source> Tradable for Bundled<T, Source>
where
    T: Tradable,
{
    type PairId = T::PairId;
    fn pair_id(&self) -> Self::PairId {
        self.0.pair_id()
    }
}

impl<T, Source, Ctx> TryFromLedger<Source, Ctx> for Bundled<T, Source>
where
    T: TryFromLedger<Source, Ctx>,
    Source: Clone,
{
    fn try_from_ledger(repr: &Source, ctx: Ctx) -> Option<Self> {
        T::try_from_ledger(&repr, ctx).map(|res| Bundled(res, repr.clone()))
    }
}
