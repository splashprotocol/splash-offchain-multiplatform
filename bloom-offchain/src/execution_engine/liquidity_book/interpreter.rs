use either::Either;

use spectrum_offchain::data::Baked;

use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::execution_effect::ExecutionEff;
use crate::execution_engine::liquidity_book::core::ExecutionRecipe;

pub trait RecipeInterpreter<Fr, Pl, Ctx, V, Bearer, Txc> {
    /// Interpret recipe [ExecutionRecipe] into a transaction candidate [Txc] and
    /// a set of new sources resulted from execution.
    fn run(
        &mut self,
        recipe: ExecutionRecipe<Fr, Pl, Bearer>,
        ctx: Ctx,
    ) -> (
        Txc,
        Vec<
            ExecutionEff<
                Bundled<Either<Baked<Fr, V>, Baked<Pl, V>>, Bearer>,
                Bundled<Either<Baked<Fr, V>, Baked<Pl, V>>, Bearer>,
            >,
        >,
    );
}
