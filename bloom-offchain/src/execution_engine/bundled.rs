use spectrum_offchain::data::{EntitySnapshot, Stable, Tradable, VersionUpdater};
use spectrum_offchain::data::order::SpecializedOrder;
use spectrum_offchain::ledger::TryFromLedger;

/// Entity bundled with its source.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Bundled<T, Bearer>(pub T, pub Bearer);

impl<T, Bearer> Bundled<T, Bearer> {
    pub fn map<T2, F>(self, f: F) -> Bundled<T2, Bearer>
    where
        F: FnOnce(T) -> T2,
    {
        Bundled(f(self.0), self.1)
    }
}

impl<T, Bearer> Stable for Bundled<T, Bearer>
where
    T: Stable,
{
    type StableId = T::StableId;
    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
}

impl<T, Bearer> SpecializedOrder for Bundled<T, Bearer>
where
    T: SpecializedOrder,
{
    type TOrderId = T::TOrderId;
    type TPoolId = T::TPoolId;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.0.get_self_ref()
    }

    fn get_pool_ref(&self) -> Self::TPoolId {
        self.0.get_pool_ref()
    }
}

impl<T, Bearer> EntitySnapshot for Bundled<T, Bearer>
where
    T: EntitySnapshot,
{
    type Version = T::Version;
    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

impl<T, Bearer> VersionUpdater for Bundled<T, Bearer>
where
    T: VersionUpdater,
{
    fn update_version(&mut self, new_version: Self::Version) {
        self.0.update_version(new_version)
    }
}

impl<T, Bearer> Tradable for Bundled<T, Bearer>
where
    T: Tradable,
{
    type PairId = T::PairId;
    fn pair_id(&self) -> Self::PairId {
        self.0.pair_id()
    }
}

impl<T, Bearer, Ctx> TryFromLedger<Bearer, Ctx> for Bundled<T, Bearer>
where
    T: TryFromLedger<Bearer, Ctx>,
    Bearer: Clone,
{
    fn try_from_ledger(repr: &Bearer, ctx: Ctx) -> Option<Self> {
        T::try_from_ledger(&repr, ctx).map(|res| Bundled(res, repr.clone()))
    }
}
