use cml_chain::builders::tx_builder::SignedTxBuilder;

use bloom_offchain::execution_engine::effect::InternalEffect;
use bloom_offchain::execution_engine::interpreter::RecipeInterpreter;
use bloom_offchain::execution_engine::liquidity_book::recipe::{ExecutionRecipe, TerminalInstruction};
use spectrum_cardano_lib::protocol_params::constant_tx_builder;

pub struct CardanoRecipeInterpreter<Ctx> {
    ctx: Ctx,
}

impl<Fr, Pl, Src, Ctx> RecipeInterpreter<Fr, Pl, Src, SignedTxBuilder> for CardanoRecipeInterpreter<Ctx> {
    fn run(
        &self,
        ExecutionRecipe { terminal, remainder }: ExecutionRecipe<Fr, Pl>,
    ) -> (SignedTxBuilder, Vec<InternalEffect<Fr, Pl, Src>>) {
        let mut tx_builder = constant_tx_builder();
        for instruction in terminal {
            match instruction {
                TerminalInstruction::Fill(fill_order) => {}
                TerminalInstruction::Swap(swap_in_pool) => {}
            }
        }
        todo!()
    }
}
