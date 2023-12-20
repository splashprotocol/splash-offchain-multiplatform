use futures::future::Either;

use crate::execution_engine::liquidity_book::recipe::LinkedExecutionRecipe;

pub trait RecipeInterpreter<Fr, Pl, Ctx, Bearer, Tx> {
    /// Interpret recipe [LinkedExecutionRecipe] into a transaction [Tx] and
    /// a set of new sources resulted from execution.
    fn run(
        &mut self,
        recipe: LinkedExecutionRecipe<Fr, Pl, Bearer>,
        ctx: Ctx,
    ) -> (Tx, Vec<(Either<Fr, Pl>, Bearer)>);
}
