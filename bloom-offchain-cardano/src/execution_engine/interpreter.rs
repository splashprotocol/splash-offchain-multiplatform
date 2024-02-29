use cml_chain::address::Address;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder};
use cml_chain::plutus::RedeemerTag;
use either::Either;
use log::trace;
use spectrum_offchain_cardano::constants::POOL_EXECUTION_UNITS;
use tailcall::tailcall;
use void::Void;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::interpreter::RecipeInterpreter;
use bloom_offchain::execution_engine::liquidity_book::fragment::StateTrans;
use bloom_offchain::execution_engine::liquidity_book::recipe::{
    LinkedExecutionRecipe, LinkedFill, LinkedSwap, LinkedTerminalInstruction,
};
use bloom_offchain::execution_engine::liquidity_book::types::Lovelace;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::{FinalizedTxOut, IndexedTxOut};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::{Baked, Has};

use crate::creds::RewardAddress;
use crate::execution_engine::execution_state::ExecutionState;
use crate::execution_engine::instances::{Magnet, TypedTransactionInput};
use crate::orders::spot::SPOT_ORDER_N2T_EX_UNITS;

/// A short-living interpreter.
#[derive(Debug, Copy, Clone)]
pub struct CardanoRecipeInterpreter;

impl<'a, Fr, Pl, Ctx> RecipeInterpreter<Fr, Pl, Ctx, OutputRef, FinalizedTxOut, SignedTxBuilder>
    for CardanoRecipeInterpreter
where
    Fr: Copy + std::fmt::Debug,
    Pl: Copy + std::fmt::Debug,
    Magnet<LinkedFill<Fr, FinalizedTxOut>>:
        BatchExec<ExecutionState, (TypedTransactionInput, Option<IndexedTxOut>), Ctx, Void>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>:
        BatchExec<ExecutionState, (TypedTransactionInput, IndexedTxOut), Ctx, Void>,
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
        let (ExecutionState { mut tx_builder, .. }, (mut indexed_outputs, mut indexed_tx_inputs), ctx) =
            execute(ctx, state, (vec![], vec![]), instructions);
        trace!(target: "offchain", "CardanoRecipeInterpreter:: execute done");

        // Collect input transaction hashes to
        indexed_tx_inputs.sort_by_key(|a| a.get_inner());
        trace!(target: "offchain", "# tx_inputs: {}", indexed_tx_inputs.len());
        for (ix, typed_tx_input) in indexed_tx_inputs.into_iter().enumerate() {
            match typed_tx_input {
                TypedTransactionInput::CFMMPool(_) => {
                    tx_builder.set_exunits(
                        RedeemerWitnessKey::new(RedeemerTag::Spend, ix as u64),
                        POOL_EXECUTION_UNITS,
                    );
                }
                TypedTransactionInput::SpotOrder(_) => {
                    tx_builder.set_exunits(
                        RedeemerWitnessKey::new(RedeemerTag::Spend, ix as u64),
                        SPOT_ORDER_N2T_EX_UNITS,
                    );
                }
            }
        }

        tx_builder
            .add_collateral(ctx.get_labeled::<Collateral>().into())
            .unwrap();

        // Set tx fee.
        let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
        trace!(target: "offchain", "estimated tx_fee: {}", estimated_tx_fee);
        tx_builder.set_fee(estimated_tx_fee + TX_FEE_CORRECTION);

        let execution_fee_address: Address = ctx.get_labeled::<RewardAddress>().into();
        // Build tx, change is execution fee.
        let tx_body = tx_builder
            .clone()
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap()
            .body();

        // Map finalized outputs to states of corresponding domain entities.
        let mut finalized_outputs = vec![];
        let tx_hash = hash_transaction_canonical(&tx_body);
        while let Some((e, IndexedTxOut(output_ix, out))) = indexed_outputs.pop() {
            let out_ref = OutputRef::new(tx_hash, output_ix as u64);
            let finalized_out = FinalizedTxOut(out, out_ref);
            finalized_outputs.push((
                e.map_either(|x| Baked::new(x, out_ref), |x| Baked::new(x, out_ref)),
                finalized_out,
            ))
        }

        // Build tx, change is execution fee.
        let tx = tx_builder
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap();
        (tx, finalized_outputs)
    }
}

const TX_FEE_CORRECTION: Lovelace = 1000;

#[tailcall]
fn execute<Fr, Pl, Ctx>(
    ctx: Ctx,
    state: ExecutionState,
    mut updates_acc: (Vec<(Either<Fr, Pl>, IndexedTxOut)>, Vec<TypedTransactionInput>),
    mut rem: Vec<LinkedTerminalInstruction<Fr, Pl, FinalizedTxOut>>,
) -> (
    ExecutionState,
    (Vec<(Either<Fr, Pl>, IndexedTxOut)>, Vec<TypedTransactionInput>),
    Ctx,
)
where
    Fr: Copy,
    Pl: Copy,
    Magnet<LinkedFill<Fr, FinalizedTxOut>>:
        BatchExec<ExecutionState, (TypedTransactionInput, Option<IndexedTxOut>), Ctx, Void>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>:
        BatchExec<ExecutionState, (TypedTransactionInput, IndexedTxOut), Ctx, Void>,
    Ctx: Clone,
{
    if let Some(instruction) = rem.pop() {
        match instruction {
            LinkedTerminalInstruction::Fill(fill_order) => {
                let next_state = fill_order.next_fr;
                let (tx_builder, (indexed_tx_in, next_bearer), ctx) =
                    Magnet(fill_order).try_exec(state, ctx).unwrap();
                trace!(target: "offchain", "Executing FILL. # TX inputs: {}", tx_builder.tx_builder.num_inputs());
                let (mut updates_acc, mut typed_tx_in_acc) = updates_acc;
                typed_tx_in_acc.push(indexed_tx_in);
                if let (StateTrans::Active(next_fr), Some(next_bearer)) = (next_state, next_bearer) {
                    updates_acc.push((Either::Left(next_fr), next_bearer));
                }
                execute(ctx, tx_builder, (updates_acc, typed_tx_in_acc), rem)
            }
            LinkedTerminalInstruction::Swap(swap) => {
                let next_state = swap.transition;
                let (tx_builder, (indexed_tx_in, next_bearer), ctx) =
                    Magnet(swap).try_exec(state, ctx).unwrap();
                trace!(target: "offchain", "Executing SWAP. # TX inputs: {}", tx_builder.tx_builder.num_inputs());
                let (mut updates_acc, mut typed_tx_in_acc) = updates_acc;
                typed_tx_in_acc.push(indexed_tx_in);
                updates_acc.push((Either::Right(next_state), next_bearer));
                execute(ctx, tx_builder, (updates_acc, typed_tx_in_acc), rem)
            }
        }
    } else {
        return (state, updates_acc, ctx);
    }
}
