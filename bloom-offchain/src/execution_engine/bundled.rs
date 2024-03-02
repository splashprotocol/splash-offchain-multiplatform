use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::{EntitySnapshot, Has, Stable, Tradable, VersionUpdater};
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

impl<T, O> EntitySnapshot for Bundled<T, O>
where
    T: Stable,
    O: Has<OutputRef>,
{
    type Version = OutputRef;
    fn version(&self) -> Self::Version {
        self.1.get()
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
