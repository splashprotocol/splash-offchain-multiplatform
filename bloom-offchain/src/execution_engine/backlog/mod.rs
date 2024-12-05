use spectrum_offchain::domain::Baked;

use crate::execution_engine::bundled::Bundled;

/// Interpreter for non-trade operations like AMM deposits/redeems.
pub trait SpecializedInterpreter<Pl, Op, Ver, Txc, Bearer, Ctx> {
    fn try_run(
        &mut self,
        pool: Bundled<Pl, Bearer>,
        order: Bundled<Op, Bearer>,
        context: Ctx,
    ) -> Option<(Txc, Bundled<Baked<Pl, Ver>, Bearer>, Bundled<Op, Bearer>)>;
}
