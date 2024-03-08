use cml_chain::address::Address;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder};
use cml_chain::builders::withdrawal_builder::SingleWithdrawalBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::{PlutusData, RedeemerTag};
use cml_chain::transaction::TransactionOutput;
use either::Either;
use log::trace;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
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
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::{Baked, Has};

use crate::creds::RewardAddress;
use crate::execution_engine::execution_state::ExecutionState;
use crate::execution_engine::instances::Magnet;
use crate::orders::spot::{SpotOrderBatchValidatorRefScriptOutput, SPOT_ORDER_N2T_EX_UNITS};

use super::instances::{FillOrderResults, TxBuilderElementsFromOrder};

/// A short-living interpreter.
#[derive(Debug, Copy, Clone)]
pub struct CardanoRecipeInterpreter;

impl<'a, Fr, Pl, Ctx> RecipeInterpreter<Fr, Pl, Ctx, OutputRef, FinalizedTxOut, SignedTxBuilder>
    for CardanoRecipeInterpreter
where
    Fr: Copy + std::fmt::Debug,
    Pl: Copy + std::fmt::Debug,
    Magnet<LinkedFill<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, FillOrderResults, Ctx, Void>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, TxBuilderElementsFromOrder, Ctx, Void>,
    Ctx: Clone + Has<Collateral> + Has<RewardAddress> + Has<SpotOrderBatchValidatorRefScriptOutput>,
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
        let (_, (mut indexed_outputs, mut tx_builder_steps), ctx) =
            execute(ctx, state, (vec![], vec![]), instructions);
        trace!(target: "offchain", "CardanoRecipeInterpreter:: execute done");

        // REBUILD the TX with ordered outputs
        let mut new_tx_builder = constant_tx_builder();

        // Sort inputs by transaction hashes to properly set the redeemers and outputs
        tx_builder_steps.sort_by_key(|a| a.input.input.transaction_id);
        trace!(target: "offchain", "# tx_inputs: {}", tx_builder_steps.len());

        let mut ordered_outputs = vec![];

        for (
            ix,
            TxBuilderElementsFromOrder {
                input,
                reference_input,
                ex_units,
                output,
            },
        ) in tx_builder_steps.into_iter().enumerate()
        {
            ordered_outputs.push(output.clone());
            new_tx_builder.add_reference_input(reference_input);
            new_tx_builder.add_input(input).unwrap();
            new_tx_builder.add_output(output).unwrap();
            new_tx_builder.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Spend, ix as u64), ex_units);
        }

        // Set batch validator
        let addr = ctx.get_labeled::<RewardAddress>();
        let reward_address = cml_chain::address::RewardAddress::new(
            addr.0.network_id().unwrap(),
            addr.0.payment_cred().unwrap().clone(),
        );
        let partial_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(
                ctx.get_labeled::<SpotOrderBatchValidatorRefScriptOutput>()
                    .0
                    .output
                    .script_hash()
                    .unwrap(),
            ),
            PlutusData::new_list(vec![]), // dummy value (this validator doesn't require redeemer)
        );
        let withdrawal_result = SingleWithdrawalBuilder::new(reward_address, 0)
            .plutus_script(partial_witness, vec![])
            .unwrap();
        new_tx_builder.add_withdrawal(withdrawal_result);
        new_tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Reward, 0),
            SPOT_ORDER_N2T_EX_UNITS,
        );

        new_tx_builder
            .add_collateral(ctx.get_labeled::<Collateral>().into())
            .unwrap();

        // Set tx fee.
        let estimated_tx_fee = new_tx_builder.min_fee(true).unwrap();
        trace!(target: "offchain", "estimated tx_fee: {}", estimated_tx_fee);
        new_tx_builder.set_fee(estimated_tx_fee + TX_FEE_CORRECTION);

        let execution_fee_address: Address = ctx.get_labeled::<RewardAddress>().into();
        // Build tx, change is execution fee.
        let tx_body = new_tx_builder
            .clone()
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap()
            .body();

        // Map finalized outputs to states of corresponding domain entities.
        let mut finalized_outputs = vec![];
        let tx_hash = hash_transaction_canonical(&tx_body);
        while let Some((e, output)) = indexed_outputs.pop() {
            let output_ix = ordered_outputs
                .iter()
                .position(|out| out.output == output)
                .unwrap();
            let out_ref = OutputRef::new(tx_hash, output_ix as u64);
            let finalized_out = FinalizedTxOut(output, out_ref);
            finalized_outputs.push((
                e.map_either(|x| Baked::new(x, out_ref), |x| Baked::new(x, out_ref)),
                finalized_out,
            ))
        }

        // Build tx, change is execution fee.
        let tx = new_tx_builder
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
    mut updates_acc: (
        Vec<(Either<Fr, Pl>, TransactionOutput)>,
        Vec<TxBuilderElementsFromOrder>,
    ),
    mut rem: Vec<LinkedTerminalInstruction<Fr, Pl, FinalizedTxOut>>,
) -> (
    ExecutionState,
    (
        Vec<(Either<Fr, Pl>, TransactionOutput)>,
        Vec<TxBuilderElementsFromOrder>,
    ),
    Ctx,
)
where
    Fr: Copy,
    Pl: Copy,
    Magnet<LinkedFill<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, FillOrderResults, Ctx, Void>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, TxBuilderElementsFromOrder, Ctx, Void>,
    Ctx: Clone,
{
    if let Some(instruction) = rem.pop() {
        match instruction {
            LinkedTerminalInstruction::Fill(fill_order) => {
                let next_state = fill_order.next_fr;
                let (
                    tx_builder,
                    FillOrderResults {
                        residual_order,
                        tx_builder_elements,
                    },
                    ctx,
                ) = Magnet(fill_order).try_exec(state, ctx).unwrap();
                trace!(target: "offchain", "Executing FILL. # TX inputs: {}", tx_builder.tx_builder.num_inputs());
                let (mut updates_acc, mut typed_tx_in_acc) = updates_acc;
                typed_tx_in_acc.push(tx_builder_elements);
                if let (StateTrans::Active(next_fr), Some(next_bearer)) = (next_state, residual_order) {
                    updates_acc.push((Either::Left(next_fr), next_bearer));
                }
                execute(ctx, tx_builder, (updates_acc, typed_tx_in_acc), rem)
            }
            LinkedTerminalInstruction::Swap(swap) => {
                let next_state = swap.transition;
                let (tx_builder, tx_builder_step_details, ctx) = Magnet(swap).try_exec(state, ctx).unwrap();
                trace!(target: "offchain", "Executing SWAP. # TX inputs: {}", tx_builder.tx_builder.num_inputs());
                let (mut updates_acc, mut typed_tx_in_acc) = updates_acc;
                updates_acc.push((
                    Either::Right(next_state),
                    tx_builder_step_details.output.output.clone(),
                ));
                typed_tx_in_acc.push(tx_builder_step_details);
                execute(ctx, tx_builder, (updates_acc, typed_tx_in_acc), rem)
            }
        }
    } else {
        return (state, updates_acc, ctx);
    }
}
