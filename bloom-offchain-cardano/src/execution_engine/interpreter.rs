use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionBuilder};

use cml_chain::transaction::TransactionOutput;
use either::Either;
use log::trace;
use num_rational::Ratio;
use tailcall::tailcall;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::execution_effect::ExecutionEff;
use bloom_offchain::execution_engine::liquidity_book::interpreter::RecipeInterpreter;
use bloom_offchain::execution_engine::liquidity_book::recipe::{
    LinkedExecutionRecipe, LinkedFill, LinkedSwap, LinkedTerminalInstruction,
};
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::data::{Baked, Has};
use spectrum_offchain_cardano::creds::OperatorRewardAddress;
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderWitnessV1;

use crate::execution_engine::execution_state::ExecutionState;
use crate::execution_engine::instances::{Magnet, OrderResult, PoolResult};

/// A short-living interpreter.
#[derive(Debug, Copy, Clone)]
pub struct CardanoRecipeInterpreter;

impl<'a, Fr, Pl, Ctx> RecipeInterpreter<Fr, Pl, Ctx, OutputRef, FinalizedTxOut, SignedTxBuilder>
    for CardanoRecipeInterpreter
where
    Fr: Copy + std::fmt::Debug,
    Pl: Copy + std::fmt::Debug,
    Magnet<LinkedFill<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, OrderResult<Fr>, Ctx>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, PoolResult<Pl>, Ctx>,
    Ctx: Clone
        + Sized
        + Has<Collateral>
        + Has<NetworkId>
        + Has<OperatorRewardAddress>
        + Has<DeployedValidator<{ LimitOrderWitnessV1 as u8 }>>,
{
    fn run(
        &mut self,
        LinkedExecutionRecipe(instructions): LinkedExecutionRecipe<Fr, Pl, FinalizedTxOut>,
        ctx: Ctx,
    ) -> (
        SignedTxBuilder,
        Vec<
            ExecutionEff<
                Bundled<Either<Baked<Fr, OutputRef>, Baked<Pl, OutputRef>>, FinalizedTxOut>,
                Bundled<Baked<Fr, OutputRef>, FinalizedTxOut>,
            >,
        >,
    ) {
        trace!("Running recipe: {:?}", instructions);
        let (mut tx_builder, effects, ctx) = execute_recipe(ctx, instructions);
        let execution_fee_address = ctx.select::<OperatorRewardAddress>().into();
        // Build tx, change is execution fee.
        let tx = tx_builder
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap();
        let tx_body_cloned = tx.body();
        let tx_hash = hash_transaction_canonical(&tx_body_cloned);

        // Map finalized outputs to states of corresponding domain entities.
        let mut finalized_effects = vec![];
        for eff in effects {
            finalized_effects.push(eff.bimap(
                |u| {
                    let output_ix = tx_body_cloned
                        .outputs
                        .iter()
                        .position(|out| out == &u.1)
                        .expect("Tx.outputs must be coherent with effects!");
                    let out_ref = OutputRef::new(tx_hash, output_ix as u64);
                    u.map(|inner| {
                        inner.map_either(|lh| Baked::new(lh, out_ref), |rh| Baked::new(rh, out_ref))
                    })
                    .map_bearer(|out| FinalizedTxOut(out, out_ref))
                },
                |e| {
                    let consumed_out_ref = e.1 .1;
                    e.map(|fr| Baked::new(fr, consumed_out_ref))
                },
            ))
        }
        trace!("Finished Tx: {}", tx_hash);
        (tx, finalized_effects)
    }
}

#[tailcall]
fn execute_recipe<Fr, Pl, Ctx>(
    ctx: Ctx,
    instructions: Vec<LinkedTerminalInstruction<Fr, Pl, FinalizedTxOut>>,
) -> (
    TransactionBuilder,
    Vec<ExecutionEff<Bundled<Either<Fr, Pl>, TransactionOutput>, Bundled<Fr, FinalizedTxOut>>>,
    Ctx,
)
where
    Fr: Copy,
    Pl: Copy,
    Magnet<LinkedFill<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, OrderResult<Fr>, Ctx>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, PoolResult<Pl>, Ctx>,
    Ctx: Clone
        + Sized
        + Has<Collateral>
        + Has<NetworkId>
        + Has<OperatorRewardAddress>
        + Has<DeployedValidator<{ LimitOrderWitnessV1 as u8 }>>,
{
    let state = ExecutionState::new();
    let (
        ExecutionState {
            tx_blueprint,
            reserved_fee,
        },
        effects,
        ctx,
    ) = execute(ctx, state, Vec::new(), instructions.clone());
    trace!("Going to interpret blueprint: {}", tx_blueprint);
    let mut tx_builder = tx_blueprint.project_onto_builder(constant_tx_builder(), ctx.select::<NetworkId>());
    tx_builder
        .add_collateral(ctx.select::<Collateral>().into())
        .unwrap();

    let estimated_fee = tx_builder.min_fee(true).unwrap();
    let fee_mismatch = reserved_fee as i64 - estimated_fee as i64;
    if fee_mismatch != 0 {
        let fee_rescale_factor = Ratio::new(estimated_fee, reserved_fee);
        let corrected_recipe = balance_fee(fee_mismatch, fee_rescale_factor, instructions);
        execute_recipe(ctx, corrected_recipe)
    } else {
        (tx_builder, effects, ctx)
    }
}

fn balance_fee<Fr, Pl, Bearer>(
    mut fee_mismatch: i64,
    rescale_factor: Ratio<u64>,
    mut instructions: Vec<LinkedTerminalInstruction<Fr, Pl, Bearer>>,
) -> Vec<LinkedTerminalInstruction<Fr, Pl, Bearer>> {
    for i in &mut instructions {
        let delta = i.scale_fee(rescale_factor);
        fee_mismatch += delta;
    }
    for i in &mut instructions {
        if fee_mismatch != 0 {
            let delta = i.correct_fee(-fee_mismatch);
            fee_mismatch += delta;
        } else {
            break;
        }
    }
    instructions
}

#[tailcall]
fn execute<Fr, Pl, Ctx>(
    ctx: Ctx,
    state: ExecutionState,
    mut updates_acc: Vec<
        ExecutionEff<Bundled<Either<Fr, Pl>, TransactionOutput>, Bundled<Fr, FinalizedTxOut>>,
    >,
    mut rem: Vec<LinkedTerminalInstruction<Fr, Pl, FinalizedTxOut>>,
) -> (
    ExecutionState,
    Vec<ExecutionEff<Bundled<Either<Fr, Pl>, TransactionOutput>, Bundled<Fr, FinalizedTxOut>>>,
    Ctx,
)
where
    Fr: Copy,
    Pl: Copy,
    Magnet<LinkedFill<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, OrderResult<Fr>, Ctx>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, PoolResult<Pl>, Ctx>,
    Ctx: Clone,
{
    if let Some(instruction) = rem.pop() {
        match instruction {
            LinkedTerminalInstruction::Fill(fill_order) => {
                let (state, result, ctx) = Magnet(fill_order).exec(state, ctx);
                updates_acc.push(result.map(|u| u.map(Either::Left)));
                execute(ctx, state, updates_acc, rem)
            }
            LinkedTerminalInstruction::Swap(swap) => {
                let (state, result, ctx) = Magnet(swap).exec(state, ctx);
                updates_acc.push(ExecutionEff::Updated(result.map(Either::Right)));
                execute(ctx, state, updates_acc, rem)
            }
        }
    } else {
        return (state, updates_acc, ctx);
    }
}

mod tests {
    use crate::execution_engine::interpreter::balance_fee;
    use bloom_offchain::execution_engine::bundled::Bundled;
    use bloom_offchain::execution_engine::liquidity_book::fragment::StateTrans;
    use bloom_offchain::execution_engine::liquidity_book::recipe::{
        Fill, LinkedFill, LinkedSwap, LinkedTerminalInstruction, TerminalInstruction,
    };
    use bloom_offchain::execution_engine::liquidity_book::side::SideM;
    use num_rational::Ratio;

    #[test]
    fn fee_overuse_balancing() {
        let instructions = vec![
            LinkedTerminalInstruction::Fill(LinkedFill {
                target_fr: Bundled((), ()),
                next_fr: StateTrans::EOL,
                removed_input: 0,
                added_output: 0,
                budget_used: 0,
                fee_used: 1_000,
            }),
            LinkedTerminalInstruction::Swap(LinkedSwap {
                target: Bundled((), ()),
                transition: (),
                side: SideM::Bid,
                input: 0,
                output: 0,
            }),
            LinkedTerminalInstruction::Fill(LinkedFill {
                target_fr: Bundled((), ()),
                next_fr: StateTrans::EOL,
                removed_input: 0,
                added_output: 0,
                budget_used: 0,
                fee_used: 2_000,
            }),
        ];
        let reserved_fee = 3_000;
        let estimated_fee = 2_000;
        let rescale_factor = Ratio::new(estimated_fee, reserved_fee);
        let fee_mismatch = reserved_fee as i64 - estimated_fee as i64;
        let balanced_instructions = balance_fee(fee_mismatch, rescale_factor, instructions);
        assert_eq!(
            balanced_instructions
                .iter()
                .map(|i| match i {
                    LinkedTerminalInstruction::Fill(f) => f.fee_used,
                    LinkedTerminalInstruction::Swap(_) => 0,
                })
                .sum::<u64>(),
            estimated_fee
        )
    }

    #[test]
    fn fee_underuse_balancing() {
        let instructions = vec![
            LinkedTerminalInstruction::Fill(LinkedFill {
                target_fr: Bundled((), ()),
                next_fr: StateTrans::EOL,
                removed_input: 0,
                added_output: 0,
                budget_used: 0,
                fee_used: 1_000,
            }),
            LinkedTerminalInstruction::Swap(LinkedSwap {
                target: Bundled((), ()),
                transition: (),
                side: SideM::Bid,
                input: 0,
                output: 0,
            }),
            LinkedTerminalInstruction::Fill(LinkedFill {
                target_fr: Bundled((), ()),
                next_fr: StateTrans::EOL,
                removed_input: 0,
                added_output: 0,
                budget_used: 0,
                fee_used: 2_000,
            }),
        ];
        let reserved_fee = 3_000;
        let estimated_fee = 4_000;
        let rescale_factor = Ratio::new(estimated_fee, reserved_fee);
        let fee_mismatch = reserved_fee as i64 - estimated_fee as i64;
        let balanced_instructions = balance_fee(fee_mismatch, rescale_factor, instructions);
        assert_eq!(
            balanced_instructions
                .iter()
                .map(|i| match i {
                    LinkedTerminalInstruction::Fill(f) => f.fee_used,
                    LinkedTerminalInstruction::Swap(_) => 0,
                })
                .sum::<u64>(),
            estimated_fee
        )
    }
}
