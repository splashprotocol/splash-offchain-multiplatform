use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionBuilder};
use futures::future::Either;
use tailcall::tailcall;
use void::Void;

use bloom_offchain::execution_engine::exec::BatchExec;
use bloom_offchain::execution_engine::interpreter::RecipeInterpreter;
use bloom_offchain::execution_engine::liquidity_book::fragment::StateTrans;
use bloom_offchain::execution_engine::liquidity_book::recipe::{
    LinkedExecutionRecipe, LinkedFill, LinkedSwap, LinkedTerminalInstruction,
};
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::{FinalizedTxOut, IndexedTxOut};
use spectrum_cardano_lib::OutputRef;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_offchain::data::Has;

use crate::operator_address::OperatorAddress;

/// A short-living interpreter.
pub struct CardanoRecipeInterpreter;

impl<'a, Fr, Pl, Ctx> RecipeInterpreter<Fr, Pl, Ctx, FinalizedTxOut, SignedTxBuilder>
    for CardanoRecipeInterpreter
where
    Fr: Copy,
    Pl: Copy,
    LinkedFill<Fr, FinalizedTxOut>: BatchExec<TransactionBuilder, Option<IndexedTxOut>, Ctx, Void>,
    LinkedSwap<Pl, FinalizedTxOut>: BatchExec<TransactionBuilder, IndexedTxOut, Ctx, Void>,
    Ctx: Clone + Has<Collateral> + Has<OperatorAddress>,
{
    fn run(
        &mut self,
        LinkedExecutionRecipe(instructions): LinkedExecutionRecipe<Fr, Pl, FinalizedTxOut>,
        ctx: Ctx,
    ) -> (SignedTxBuilder, Vec<(Either<Fr, Pl>, FinalizedTxOut)>) {
        let tx_builder = constant_tx_builder();
        let (mut tx_builder, mut indexed_outputs, ctx) = execute(ctx, tx_builder, vec![], instructions);
        tx_builder.add_collateral(ctx.get::<Collateral>().into()).unwrap();
        let tx = tx_builder
            .build(ChangeSelectionAlgo::Default, &ctx.get::<OperatorAddress>().into())
            .unwrap();
        let mut finalized_outputs = vec![];
        let tx_hash = hash_transaction_canonical(&tx.body());
        while let Some((sid, IndexedTxOut(ix, out))) = indexed_outputs.pop() {
            let out_ref = OutputRef::new(tx_hash, ix as u64);
            let finalized_out = FinalizedTxOut(out, out_ref);
            finalized_outputs.push((sid, finalized_out))
        }
        (tx, finalized_outputs)
    }
}

#[tailcall]
fn execute<Fr, Pl, Ctx>(
    ctx: Ctx,
    tx_builder: TransactionBuilder,
    mut updates_acc: Vec<(Either<Fr, Pl>, IndexedTxOut)>,
    mut rem: Vec<LinkedTerminalInstruction<Fr, Pl, FinalizedTxOut>>,
) -> (TransactionBuilder, Vec<(Either<Fr, Pl>, IndexedTxOut)>, Ctx)
where
    Fr: Copy,
    Pl: Copy,
    LinkedFill<Fr, FinalizedTxOut>: BatchExec<TransactionBuilder, Option<IndexedTxOut>, Ctx, Void>,
    LinkedSwap<Pl, FinalizedTxOut>: BatchExec<TransactionBuilder, IndexedTxOut, Ctx, Void>,
    Ctx: Clone,
{
    if let Some(instruction) = rem.pop() {
        match instruction {
            LinkedTerminalInstruction::Fill(fill_order) => {
                let next_state = fill_order.transition;
                let (tx_builder, next_bearer, ctx) = fill_order.try_exec(tx_builder, ctx).unwrap();
                if let (StateTrans::Active(next_fr), Some(next_bearer)) = (next_state, next_bearer) {
                    updates_acc.push((Either::Left(next_fr), next_bearer));
                }
                execute(ctx, tx_builder, updates_acc, rem)
            }
            LinkedTerminalInstruction::Swap(swap) => {
                let next_state = swap.transition;
                let (tx_builder, next_bearer, ctx) = swap.try_exec(tx_builder, ctx).unwrap();
                updates_acc.push((Either::Right(next_state), next_bearer));
                execute(ctx, tx_builder, updates_acc, rem)
            }
        }
    } else {
        return (tx_builder, updates_acc, ctx);
    }
}
