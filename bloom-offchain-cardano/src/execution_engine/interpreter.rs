use cml_chain::address::Address;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder};
use cml_chain::transaction::TransactionOutput;
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
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::{Baked, Has};
use spectrum_offchain_cardano::constants::MIN_SAFE_ADA_VALUE;

use crate::execution_engine::execution_state::ExecutionState;
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
    Magnet<LinkedFill<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, Option<IndexedTxOut>, Ctx, Void>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, IndexedTxOut, Ctx, Void>,
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
        let state = ExecutionState::new();
        let (
            ExecutionState {
                mut tx_builder,
                executor_fee_acc,
            },
            mut indexed_outputs,
            ctx,
        ) = execute(ctx, state, vec![], instructions);
        let reward_address: Address = ctx.get::<RewardAddress>().into();
        if executor_fee_acc.coin >= MIN_SAFE_ADA_VALUE {
            let fee_accumulator_output = TransactionOutput::new(reward_address, executor_fee_acc, None, None);
            tx_builder
                .add_output(SingleOutputBuilderResult::new(fee_accumulator_output))
                .unwrap();
        }
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
    state: ExecutionState,
    mut updates_acc: Vec<(Either<Fr, Pl>, IndexedTxOut)>,
    mut rem: Vec<LinkedTerminalInstruction<Fr, Pl, FinalizedTxOut>>,
) -> (ExecutionState, Vec<(Either<Fr, Pl>, IndexedTxOut)>, Ctx)
where
    Fr: Copy,
    Pl: Copy,
    Magnet<LinkedFill<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, Option<IndexedTxOut>, Ctx, Void>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, IndexedTxOut, Ctx, Void>,
    Ctx: Clone,
{
    if let Some(instruction) = rem.pop() {
        match instruction {
            LinkedTerminalInstruction::Fill(fill_order) => {
                let next_state = fill_order.transition;
                let (tx_builder, next_bearer, ctx) = Magnet(fill_order).try_exec(state, ctx).unwrap();
                if let (StateTrans::Active(next_fr), Some(next_bearer)) = (next_state, next_bearer) {
                    updates_acc.push((Either::Left(next_fr), next_bearer));
                }
                execute(ctx, tx_builder, updates_acc, rem)
            }
            LinkedTerminalInstruction::Swap(swap) => {
                let next_state = swap.transition;
                let (tx_builder, next_bearer, ctx) = Magnet(swap).try_exec(state, ctx).unwrap();
                updates_acc.push((Either::Right(next_state), next_bearer));
                execute(ctx, tx_builder, updates_acc, rem)
            }
        }
    } else {
        return (state, updates_acc, ctx);
    }
}
