use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use spectrum_offchain::backlog;
use spectrum_offchain::domain::order::SpecializedOrder;
use spectrum_offchain::domain::{EntitySnapshot, SeqState, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;

use crate::execution_engine::liquidity_book;

/// Entity bundled with its source.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Bundled<T, Bearer>(pub T, pub Bearer);

impl<T: Display, B> Display for Bundled<T, B> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("Bundled({}, _)", self.0).as_str())
    }
}

impl<T, Bearer> Bundled<T, Bearer> {
    pub fn map<T2, F>(self, f: F) -> Bundled<T2, Bearer>
    where
        F: FnOnce(T) -> T2,
    {
        Bundled(f(self.0), self.1)
    }
    pub fn map_bearer<B2, F>(self, f: F) -> Bundled<T, B2>
    where
        F: FnOnce(Bearer) -> B2,
    {
        Bundled(self.0, f(self.1))
    }
    pub fn inspect<F>(self, f: F)
    where
        F: FnOnce(&T),
    {
        f(&self.0)
    }
}

impl<T, Bearer> Hash for Bundled<T, Bearer>
where
    T: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state)
    }
}

impl<T, U, Bearer> liquidity_book::weight::Weighted<U> for Bundled<T, Bearer>
where
    T: liquidity_book::weight::Weighted<U>,
{
    fn weight(&self) -> liquidity_book::weight::OrderWeight<U> {
        self.0.weight()
    }
}

impl<T, Bearer> backlog::data::Weighted for Bundled<T, Bearer>
where
    T: backlog::data::Weighted,
{
    fn weight(&self) -> backlog::data::OrderWeight {
        self.0.weight()
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
    fn is_quasi_permanent(&self) -> bool {
        self.0.is_quasi_permanent()
    }
}

impl<T, Bearer> SeqState for Bundled<T, Bearer>
where
    T: SeqState,
{
    fn is_initial(&self) -> bool {
        self.0.is_initial()
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
    fn try_from_ledger(repr: &Bearer, ctx: &Ctx) -> Option<Self> {
        T::try_from_ledger(&repr, ctx).map(|res| Bundled(res, repr.clone()))
    }
}
