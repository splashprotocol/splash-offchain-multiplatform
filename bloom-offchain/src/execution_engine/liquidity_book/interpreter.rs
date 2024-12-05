use either::Either;
use spectrum_offchain::domain::Baked;

use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::execution_effect::ExecutionEff;
use crate::execution_engine::funding_effect::FundingIO;
use crate::execution_engine::liquidity_book::core::ExecutionRecipe;

pub struct ExecutionResult<Fr, Pl, V, Bearer, Txc> {
    pub txc: Txc,
    pub matchmaking_effects: Vec<
        ExecutionEff<
            Bundled<Either<Baked<Fr, V>, Baked<Pl, V>>, Bearer>,
            Bundled<Either<Baked<Fr, V>, Baked<Pl, V>>, Bearer>,
        >,
    >,
    /// Result of funding usage.
    pub funding_io: FundingIO<Bearer, Bearer>,
}

pub trait RecipeInterpreter<Fr, Pl, Ctx, V, Bearer, Txc> {
    /// Interpret recipe [ExecutionRecipe] into a transaction candidate [Txc] and
    /// a set of new sources resulted from execution.
    fn run(
        &mut self,
        recipe: ExecutionRecipe<Fr, Pl, Bearer>,
        funding: Bearer,
        ctx: Ctx,
    ) -> ExecutionResult<Fr, Pl, V, Bearer, Txc>;
}
