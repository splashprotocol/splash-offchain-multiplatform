use crate::execution_engine::effect::InternalEffect;
use crate::execution_engine::liquidity_book::recipe::ExecutionRecipe;

pub trait RecipeInterpreter<Fr, Pl, Src, Tx> {
    /// Interpret recipe [ExecutionRecipe] into transaction [Tx] and
    /// series of internal effects [InternalEffect] resulted from execution.
    fn run(&mut self, recipe: ExecutionRecipe<Fr, Pl>) -> (Tx, Vec<InternalEffect<Fr, Pl, Src>>);
}
