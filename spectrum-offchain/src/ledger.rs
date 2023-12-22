/// Tries to read domain entity from on-chain representation (e.g. a UTxO).
pub trait TryFromLedger<Repr, Ctx>: Sized {
    fn try_from_ledger(repr: Repr, ctx: Ctx) -> Result<Self, Repr>;
}

pub fn try_parse<Repr, Ctx, T, F: FnOnce(&Repr, Ctx) -> Option<T>>(
    repr: Repr,
    ctx: Ctx,
    parse: F,
) -> Result<T, Repr> {
    match parse(&repr, ctx) {
        None => Err(repr),
        Some(r) => Ok(r),
    }
}

/// Encodes domain entity into on-chain representation.
pub trait IntoLedger<Repr, Ctx> {
    fn into_ledger(self, ctx: Ctx) -> Repr;
}
