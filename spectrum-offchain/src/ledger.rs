/// Tries to read domain entity from on-chain representation (e.g. a UTxO).
pub trait TryFromLedger<Repr, Ctx>: Sized {
    fn try_from_ledger(repr: Repr, ctx: Ctx) -> Option<Self>;
}
