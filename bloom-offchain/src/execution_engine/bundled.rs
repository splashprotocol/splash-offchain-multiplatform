use spectrum_offchain::data::EntitySnapshot;
use spectrum_offchain::ledger::TryFromLedger;

/// Entity bundled with its source.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Bundled<T, Source>(pub T, pub Source);

impl<T, Source> EntitySnapshot for Bundled<T, Source>
where
    T: EntitySnapshot,
{
    type StableId = T::StableId;
    type Version = T::Version;
    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
    fn version(&self) -> Self::Version {
        self.0.version()
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
