use cml_chain::address::Address;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder};
use cml_chain::builders::withdrawal_builder::SingleWithdrawalBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::{PlutusData, RedeemerTag};
use cml_chain::transaction::TransactionOutput;
use either::Either;
use log::trace;
use tailcall::tailcall;
use void::Void;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::execution_effect::ExecutionEff;
use bloom_offchain::execution_engine::liquidity_book::interpreter::RecipeInterpreter;
use bloom_offchain::execution_engine::liquidity_book::recipe::{
    LinkedExecutionRecipe, LinkedFill, LinkedSwap, LinkedTerminalInstruction,
};
use bloom_offchain::execution_engine::liquidity_book::types::Lovelace;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::{Baked, Has};
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderWitness;

use crate::creds::RewardAddress;
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
    Magnet<LinkedFill<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, OrderResult<Fr>, Ctx, Void>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, PoolResult<Pl>, Ctx, Void>,
    Ctx: Clone + Has<Collateral> + Has<RewardAddress> + Has<DeployedValidator<{ LimitOrderWitness as u8 }>>,
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
        let state = ExecutionState::new();
        let (
            ExecutionState {
                tx_blueprint,
                execution_budget_acc,
            },
            effects,
            ctx,
        ) = execute(ctx, state, Vec::new(), instructions);

        let mut tx_builder = tx_blueprint.apply_to_builder(constant_tx_builder());

        // Set batch validator
        let addr = ctx.get_labeled::<RewardAddress>();
        let reward_address = cml_chain::address::RewardAddress::new(
            addr.0.network_id().unwrap(),
            addr.0.payment_cred().unwrap().clone(),
        );
        let order_witness = ctx.get_labeled::<DeployedValidator<{ LimitOrderWitness as u8 }>>();
        let partial_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(order_witness.reference_utxo.output.script_hash().unwrap()),
            PlutusData::new_list(vec![]), // dummy value (this validator doesn't require redeemer)
        );
        let withdrawal_result = SingleWithdrawalBuilder::new(reward_address, 0)
            .plutus_script(partial_witness, vec![])
            .unwrap();
        tx_builder.add_withdrawal(withdrawal_result);
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Reward, 0),
            order_witness.ex_budget.into(),
        );
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

        let tx_hash = hash_transaction_canonical(&tx_body);

        // Map finalized outputs to states of corresponding domain entities.
        let mut finalized_effects = vec![];
        for eff in effects {
            finalized_effects.push(eff.bimap(
                |u| {
                    let output_ix = tx_body
                        .outputs
                        .iter()
                        .position(|out| out == &u.1)
                        .expect("Tx.output must be coherent with effects!");
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

        // Build tx, change is execution fee.
        let tx = tx_builder
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap();
        (tx, finalized_effects)
    }
}

const TX_FEE_CORRECTION: Lovelace = 1000;

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
    Magnet<LinkedFill<Fr, FinalizedTxOut>>: BatchExec<ExecutionState, OrderResult<Fr>, Ctx, Void>,
    Magnet<LinkedSwap<Pl, FinalizedTxOut>>: BatchExec<ExecutionState, PoolResult<Pl>, Ctx, Void>,
    Ctx: Clone,
{
    if let Some(instruction) = rem.pop() {
        match instruction {
            LinkedTerminalInstruction::Fill(fill_order) => {
                let (state, result, ctx) = Magnet(fill_order).try_exec(state, ctx).unwrap();
                updates_acc.push(result.map(|u| u.map(Either::Left)));
                execute(ctx, state, updates_acc, rem)
            }
            LinkedTerminalInstruction::Swap(swap) => {
                let (state, result, ctx) = Magnet(swap).try_exec(state, ctx).unwrap();
                updates_acc.push(ExecutionEff::Updated(result.map(Either::Right)));
                execute(ctx, state, updates_acc, rem)
            }
        }
    } else {
        return (state, updates_acc, ctx);
    }
}
