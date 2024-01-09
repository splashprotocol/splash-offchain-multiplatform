use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionBuilder};
use either::Either;
use tailcall::tailcall;
use void::Void;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::interpreter::RecipeInterpreter;
use bloom_offchain::execution_engine::liquidity_book::fragment::StateTrans;
use bloom_offchain::execution_engine::liquidity_book::recipe::{
    LinkedExecutionRecipe, LinkedFill, LinkedSwap, LinkedTerminalInstruction,
};
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::{FinalizedTxOut, IndexedTxOut};
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::{Baked, Has};

use crate::execution_engine::instances::Magnet;
use crate::operator_address::RewardAddress;

/// A short-living interpreter.
#[derive(Debug, Copy, Clone)]
pub struct CardanoRecipeInterpreter;

impl<'a, Fr, Pl, Ctx> RecipeInterpreter<Fr, Pl, Ctx, OutputRef, FinalizedTxOut, SignedTxBuilder>
    for CardanoRecipeInterpreter
where
    Fr: Copy,
    Pl: Copy,
    Magnet<LinkedFill<Fr, FinalizedTxOut>>: BatchExec<TransactionBuilder, Option<IndexedTxOut>, Ctx, Void>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>: BatchExec<TransactionBuilder, IndexedTxOut, Ctx, Void>,
    Ctx: Clone + Has<Collateral> + Has<RewardAddress>,
{
    fn run(
        &mut self,
        LinkedExecutionRecipe(instructions): LinkedExecutionRecipe<Fr, Pl, FinalizedTxOut>,
        ctx: Ctx,
    ) -> (
        SignedTxBuilder,
        Vec<(Either<Baked<Fr, OutputRef>, Baked<Pl, OutputRef>>, FinalizedTxOut)>,
    ) {
        let tx_builder = constant_tx_builder();
        let (mut tx_builder, mut indexed_outputs, ctx) = execute(ctx, tx_builder, vec![], instructions);
        tx_builder.add_collateral(ctx.get::<Collateral>().into()).unwrap();
        let tx = tx_builder
            .build(ChangeSelectionAlgo::Default, &ctx.get::<RewardAddress>().into())
            .unwrap();
        let mut finalized_outputs = vec![];
        let tx_hash = hash_transaction_canonical(&tx.body());
        while let Some((e, IndexedTxOut(ix, out))) = indexed_outputs.pop() {
            let out_ref = OutputRef::new(tx_hash, ix as u64);
            let finalized_out = FinalizedTxOut(out, out_ref);
            finalized_outputs.push((
                e.map_either(|x| Baked::new(x, out_ref), |x| Baked::new(x, out_ref)),
                finalized_out,
            ))
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
    Magnet<LinkedFill<Fr, FinalizedTxOut>>: BatchExec<TransactionBuilder, Option<IndexedTxOut>, Ctx, Void>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>: BatchExec<TransactionBuilder, IndexedTxOut, Ctx, Void>,
    Ctx: Clone,
{
    if let Some(instruction) = rem.pop() {
        match instruction {
            LinkedTerminalInstruction::Fill(fill_order) => {
                let next_state = fill_order.transition;
                let (tx_builder, next_bearer, ctx) = Magnet(fill_order).try_exec(tx_builder, ctx).unwrap();
                if let (StateTrans::Active(next_fr), Some(next_bearer)) = (next_state, next_bearer) {
                    updates_acc.push((Either::Left(next_fr), next_bearer));
                }
                execute(ctx, tx_builder, updates_acc, rem)
            }
            LinkedTerminalInstruction::Swap(swap) => {
                let next_state = swap.transition;
                let (tx_builder, next_bearer, ctx) = Magnet(swap).try_exec(tx_builder, ctx).unwrap();
                updates_acc.push((Either::Right(next_state), next_bearer));
                execute(ctx, tx_builder, updates_acc, rem)
            }
        }
    } else {
        return (tx_builder, updates_acc, ctx);
    }
}
