use either::Either;
use spectrum_offchain::data::Baked;

use crate::execution_engine::liquidity_book::recipe::LinkedExecutionRecipe;

pub trait RecipeInterpreter<Fr, Pl, Ctx, V, Bearer, Tx> {
    /// Interpret recipe [LinkedExecutionRecipe] into a transaction [Tx] and
    /// a set of new sources resulted from execution.
    fn run(
        &mut self,
        recipe: LinkedExecutionRecipe<Fr, Pl, Bearer, V>,
        ctx: Ctx,
    ) -> (Tx, Vec<(Either<Baked<Fr>, Baked<Pl>>, Bearer)>);
}
