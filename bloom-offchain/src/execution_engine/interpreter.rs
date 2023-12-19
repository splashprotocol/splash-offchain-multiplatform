use crate::execution_engine::liquidity_book::recipe::{ExecutionRecipe, LinkedExecutionRecipe};
use crate::execution_engine::StableId;

pub trait RecipeInterpreter<Fr, Pl, Ctx, Bearer, Tx> {
    /// Interpret recipe [ExecutionRecipe] into a transaction [Tx] and
    /// a set of new sources resulted from execution.
    fn run(&mut self, recipe: LinkedExecutionRecipe<Fr, Pl, Bearer>, ctx: Ctx) -> (Tx, Vec<(StableId, Bearer)>);
}
