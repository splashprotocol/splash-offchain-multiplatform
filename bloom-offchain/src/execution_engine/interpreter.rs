use crate::execution_engine::liquidity_book::recipe::ExecutionRecipe;
use crate::execution_engine::StableId;

pub trait RecipeInterpreter<Fr, Pl, Ctx, Src, Tx> {
    /// Interpret recipe [ExecutionRecipe] into a transaction [Tx] and
    /// a set of new sources resulted from execution.
    fn run(&mut self, recipe: ExecutionRecipe<Fr, Pl>, ctx: Ctx) -> (Tx, Vec<(StableId, Src)>);
}
