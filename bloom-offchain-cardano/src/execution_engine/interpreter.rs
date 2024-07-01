use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionBuilder};

use either::Either;
use log::trace;
use num_rational::Ratio;
use tailcall::tailcall;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::liquidity_book::core::{Execution, ExecutionRecipe, Make, Take};
use bloom_offchain::execution_engine::liquidity_book::fragment::{Fragment, TakerBehaviour};
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
use spectrum_offchain_cardano::creds::{OperatorCred, OperatorRewardAddress};
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderWitnessV1;

use crate::execution_engine::execution_state::ExecutionState;
use crate::execution_engine::instances::{EffectPreview, FinalizedEffect, Magnet};

/// A short-living interpreter.
#[derive(Debug, Copy, Clone)]
pub struct CardanoRecipeInterpreter;

impl<'a, Fr, Pl, Ctx> RecipeInterpreter<Fr, Pl, Ctx, OutputRef, FinalizedTxOut, SignedTxBuilder>
    for CardanoRecipeInterpreter
where
    Fr: Fragment + TakerBehaviour + Copy + std::fmt::Debug,
    Pl: Copy + std::fmt::Debug,
    Magnet<Take<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Fr>, Ctx>,
    Magnet<Make<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Pl>, Ctx>,
    Ctx: Clone
        + Sized
        + Has<Collateral>
        + Has<NetworkId>
        + Has<OperatorRewardAddress>
        + Has<DeployedValidator<{ LimitOrderWitnessV1 as u8 }>>,
{
    fn run(
        &mut self,
        ExecutionRecipe(instructions): ExecutionRecipe<Fr, Pl, FinalizedTxOut>,
        ctx: Ctx,
    ) -> (
        SignedTxBuilder,
        Vec<FinalizedEffect<Either<Baked<Fr, OutputRef>, Baked<Pl, OutputRef>>>>,
    ) {
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
                |p| {
                    let output_ix = tx_body_cloned
                        .outputs
                        .iter()
                        .position(|out| out == &p.1)
                        .expect("Tx.outputs must be coherent with effects!");
                    let out_ref = OutputRef::new(tx_hash, output_ix as u64);
                    p.map(|inner| {
                        inner.map_either(|lh| Baked::new(lh, out_ref), |rh| Baked::new(rh, out_ref))
                    })
                    .map_bearer(|out| FinalizedTxOut(out, out_ref))
                },
                |c| {
                    let Bundled(_, FinalizedTxOut(_, consumed_out_ref)) = c;
                    c.map(|fr| {
                        fr.map_either(
                            |fr| Baked::new(fr, consumed_out_ref),
                            |pl| Baked::new(pl, consumed_out_ref),
                        )
                    })
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
    instructions: Vec<Execution<Fr, Pl, FinalizedTxOut>>,
) -> (TransactionBuilder, Vec<EffectPreview<Either<Fr, Pl>>>, Ctx)
where
    Fr: Fragment + TakerBehaviour + Copy,
    Pl: Copy,
    Magnet<Take<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Fr>, Ctx>,
    Magnet<Make<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Pl>, Ctx>,
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
    trace!(
        "Est. fee {}, reserved fee: {}, mismatch {}",
        estimated_fee,
        reserved_fee,
        fee_mismatch
    );
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
    mut instructions: Vec<Execution<Fr, Pl, Bearer>>,
) -> Vec<Execution<Fr, Pl, Bearer>>
where
    Fr: Fragment + TakerBehaviour + Copy,
{
    for i in &mut instructions {
        if let Either::Left(take) = i {
            take.scale_budget(rescale_factor);
        }
    }
    for i in &mut instructions {
        if let Either::Left(take) = i {
            if fee_mismatch != 0 {
                let delta = take.correct_budget(-fee_mismatch);
                fee_mismatch += delta;
            } else {
                break;
            }
        }
    }
    instructions
}

#[tailcall]
fn execute<Fr, Pl, Ctx>(
    ctx: Ctx,
    state: ExecutionState,
    mut updates_acc: Vec<EffectPreview<Either<Fr, Pl>>>,
    mut rem: Vec<Execution<Fr, Pl, FinalizedTxOut>>,
) -> (ExecutionState, Vec<EffectPreview<Either<Fr, Pl>>>, Ctx)
where
    Fr: Copy,
    Pl: Copy,
    Magnet<Take<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Fr>, Ctx>,
    Magnet<Make<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Pl>, Ctx>,
    Ctx: Clone,
{
    if let Some(instruction) = rem.pop() {
        match instruction {
            Either::Left(take) => {
                let (state, result, ctx) = Magnet(take).exec(state, ctx);
                updates_acc.push(result.bimap(|u| u.map(Either::Left), |e| e.map(Either::Left)));
                execute(ctx, state, updates_acc, rem)
            }
            Either::Right(make) => {
                let (state, result, ctx) = Magnet(make).exec(state, ctx);
                updates_acc.push(result.bimap(|u| u.map(Either::Right), |e| e.map(Either::Right)));
                execute(ctx, state, updates_acc, rem)
            }
        }
    } else {
        return (state, updates_acc, ctx);
    }
}

mod tests {

    #[test]
    fn fee_overuse_balancing() {}

    #[test]
    fn fee_underuse_balancing() {}
}
