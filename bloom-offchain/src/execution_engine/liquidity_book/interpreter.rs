use either::Either;

use spectrum_offchain::data::Baked;
use spectrum_offchain::executor::RunOrderError;

use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::execution_effect::ExecutionEff;
use crate::execution_engine::liquidity_book::recipe::LinkedExecutionRecipe;

pub trait RecipeInterpreter<Fr, Pl, Ctx, V, Bearer, Txc> {
    /// Interpret recipe [LinkedExecutionRecipe] into a transaction candidate [Txc] and
    /// a set of new sources resulted from execution.
    fn run(
        &mut self,
        recipe: LinkedExecutionRecipe<Fr, Pl, Bearer>,
        ctx: Ctx,
    ) -> Result<(
        Txc,
        Vec<ExecutionEff<Bundled<Either<Baked<Fr, V>, Baked<Pl, V>>, Bearer>, Bundled<Baked<Fr, V>, Bearer>>>,
    ), RunOrderError<u64>>;
}
