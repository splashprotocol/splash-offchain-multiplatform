use either::Either;

/// Tries to read domain entity from on-chain representation (e.g. a UTxO).
pub trait TryFromLedger<Repr, Ctx>: Sized {
    fn try_from_ledger(repr: &Repr, ctx: &Ctx) -> Option<Self>;
}

impl<A, B, Repr, Ctx> TryFromLedger<Repr, Ctx> for Either<A, B>
where
    A: TryFromLedger<Repr, Ctx>,
    B: TryFromLedger<Repr, Ctx>,
    Ctx: Clone,
{
    fn try_from_ledger(repr: &Repr, ctx: &Ctx) -> Option<Self> {
        A::try_from_ledger(repr, ctx)
            .map(|a| Either::Left(a))
            .or_else(|| B::try_from_ledger(repr, ctx).map(|b| Either::Right(b)))
    }
}

/// Encodes domain entity into on-chain representation.
pub trait IntoLedger<Repr, Ctx> {
    fn into_ledger(self, ctx: Ctx) -> Repr;
}
