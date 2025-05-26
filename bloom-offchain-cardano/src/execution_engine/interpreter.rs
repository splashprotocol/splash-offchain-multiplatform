use cml_chain::auxdata::{AuxiliaryData, ConwayFormatAuxData, Metadata, TransactionMetadatum};
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionBuilder};
use cml_chain::transaction::TransactionOutput;
use cml_core::serialization::StringEncoding;
use either::Either;
use log::trace;
use num_rational::Ratio;
use std::fmt::Debug;
use tailcall::tailcall;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::funding_effect::FundingIO;
use bloom_offchain::execution_engine::liquidity_book::core::{Execution, ExecutionRecipe, Make, Take};
use bloom_offchain::execution_engine::liquidity_book::interpreter::{ExecutionResult, RecipeInterpreter};
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::types::Lovelace;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::domain::{Baked, Has};
use spectrum_offchain_cardano::constants::ADDITIONAL_FEE;
use spectrum_offchain_cardano::creds::OperatorRewardAddress;
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderWitnessV1;

use crate::execution_engine::execution_state::ExecutionState;
use crate::execution_engine::instances::{EffectPreview, Magnet};

/// A short-living interpreter.
#[derive(Debug, Copy, Clone)]
pub struct CardanoRecipeInterpreter {
    take_residual_fee: bool,
}

impl CardanoRecipeInterpreter {
    pub fn new(take_residual_fee: bool) -> CardanoRecipeInterpreter {
        CardanoRecipeInterpreter { take_residual_fee }
    }
}

impl<'a, T, M, Ctx> RecipeInterpreter<T, M, Ctx, OutputRef, FinalizedTxOut, SignedTxBuilder>
    for CardanoRecipeInterpreter
where
    T: MarketTaker + TakerBehaviour + Copy + Debug,
    M: Copy + Debug,
    Magnet<Take<T, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<T>, Ctx>,
    Magnet<Make<M, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<M>, Ctx>,
    Ctx: Clone + Sized + Has<Collateral> + Has<NetworkId> + Has<OperatorRewardAddress>,
{
    fn run(
        &mut self,
        ExecutionRecipe(instructions): ExecutionRecipe<T, M, FinalizedTxOut>,
        funding: FinalizedTxOut,
        ctx: Ctx,
    ) -> ExecutionResult<T, M, OutputRef, FinalizedTxOut, SignedTxBuilder> {
        let (tx_builder, effects, funding_io_preview, ctx) =
            execute_recipe(funding, self.take_residual_fee, ctx, instructions, 0);

        let mut order_of_execution = vec![];
        for (execution_seq_num, eff) in effects.iter().enumerate() {
            if let EffectPreview::Updated(Bundled(Either::Left(_), utxo), _)
            | EffectPreview::Eliminated(Bundled(Either::Left(_), utxo)) = eff
            {
                let input_ix = tx_builder
                    .get_inputs()
                    .iter()
                    .position(|input| input.output == utxo.0)
                    .expect("Tx.inputs must be coherent with effects!");
                order_of_execution.push((input_ix, execution_seq_num));
            }
        }

        let execution_fee_address = ctx.select::<OperatorRewardAddress>().into();
        // Build tx, change is execution fee.
        let tx = with_metadata(tx_builder, order_of_execution)
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap();
        let tx_body = tx.body_ref();
        let tx_hash = hash_transaction_canonical(tx_body);
        let tx_outputs = &tx_body.outputs;

        // Map finalized outputs to states of corresponding domain entities.
        let mut finalized_effects = vec![];
        for eff in effects {
            finalized_effects.push(eff.bimap(
                |p| {
                    let output_ix = tx_outputs
                        .iter()
                        .position(|out| out == &p.1)
                        .expect("Tx.outputs must be coherent with effects!");
                    let out_ref = OutputRef::new(tx_hash, output_ix as u64);
                    p.map(|inner| {
                        inner.map_either(|tk| Baked::new(tk, out_ref), |mk| Baked::new(mk, out_ref))
                    })
                    .map_bearer(|out| FinalizedTxOut(out, out_ref))
                },
                |c| {
                    let Bundled(_, FinalizedTxOut(_, consumed_out_ref)) = c;
                    c.map(|fr| {
                        fr.map_either(
                            |tk| Baked::new(tk, consumed_out_ref),
                            |mk| Baked::new(mk, consumed_out_ref),
                        )
                    })
                },
            ))
        }

        let finalized_funding_io = funding_io_preview.map_output(|o| {
            let output_ix = tx_outputs
                .iter()
                .position(|out| out == &o)
                .expect("Tx.outputs must be coherent with funding IO!");
            let out_ref = OutputRef::new(tx_hash, output_ix as u64);
            FinalizedTxOut(o, out_ref)
        });

        trace!("Finished Tx: {}", tx_hash);
        ExecutionResult {
            txc: tx,
            matchmaking_effects: finalized_effects,
            funding_io: finalized_funding_io,
        }
    }
}

const ORDERING_KEY: u64 = 0;

fn with_metadata(
    mut tx_builder: TransactionBuilder,
    order_of_execution: Vec<(usize, usize)>,
) -> TransactionBuilder {
    let mut encoded_ordering = vec![];
    for (output_ix, execution_seq_num) in order_of_execution {
        encoded_ordering.push(output_ix as u8);
        encoded_ordering.push(execution_seq_num as u8);
    }
    tx_builder.add_auxiliary_data(AuxiliaryData::Conway(ConwayFormatAuxData {
        metadata: Some(Metadata {
            entries: vec![(
                ORDERING_KEY,
                TransactionMetadatum::Bytes {
                    bytes: encoded_ordering,
                    bytes_encoding: StringEncoding::Canonical,
                },
            )],
            encodings: None,
        }),
        native_scripts: None,
        plutus_v1_scripts: None,
        plutus_v2_scripts: None,
        plutus_v3_scripts: None,
        encodings: None,
    }));
    tx_builder
}

#[tailcall]
fn execute_recipe<Tk, Mk, Ctx>(
    funding: FinalizedTxOut,
    take_residual_fee: bool,
    ctx: Ctx,
    instructions: Vec<Execution<Tk, Mk, FinalizedTxOut>>,
    accumulated_residue: Lovelace,
) -> (
    TransactionBuilder,
    Vec<EffectPreview<Either<Tk, Mk>>>,
    FundingIO<FinalizedTxOut, TransactionOutput>,
    Ctx,
)
where
    Tk: MarketTaker + TakerBehaviour + Copy,
    Mk: Copy,
    Magnet<Take<Tk, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Tk>, Ctx>,
    Magnet<Make<Mk, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Mk>, Ctx>,
    Ctx: Clone + Sized + Has<Collateral> + Has<NetworkId> + Has<OperatorRewardAddress>,
{
    let state = ExecutionState::new();
    let (
        ExecutionState {
            tx_blueprint,
            reserved_tx_fee,
            operator_interest,
        },
        effects,
        ctx,
    ) = execute(ctx, state, Vec::new(), instructions.clone());
    trace!("Going to interpret blueprint: {}", tx_blueprint);
    let (mut tx_builder, funding_io) = tx_blueprint.project_onto_builder(
        constant_tx_builder(),
        ctx.select::<NetworkId>(),
        ctx.select::<OperatorRewardAddress>(),
        funding.clone(),
        operator_interest + accumulated_residue,
    );
    tx_builder
        .add_collateral(ctx.select::<Collateral>().into())
        .unwrap();

    let estimated_fee = tx_builder.min_fee(true).unwrap() + ADDITIONAL_FEE;
    let updated_tx_fee = reserved_tx_fee - accumulated_residue;
    let fee_mismatch = updated_tx_fee as i64 - estimated_fee as i64;
    trace!(
        "Est. fee: {}, reserved fee: {}, mismatch: {}",
        estimated_fee,
        reserved_tx_fee,
        fee_mismatch
    );
    if fee_mismatch != 0 {
        if take_residual_fee && fee_mismatch > 0 {
            let accumulated_residue = accumulated_residue + fee_mismatch.unsigned_abs();
            execute_recipe(funding, false, ctx, instructions, accumulated_residue)
        } else {
            let fee_rescale_factor = Ratio::new(estimated_fee, reserved_tx_fee);
            let corrected_recipe = balance_fee(fee_mismatch, fee_rescale_factor, instructions);
            execute_recipe(funding, false, ctx, corrected_recipe, 0)
        }
    } else {
        (tx_builder, effects, funding_io, ctx)
    }
}

fn balance_fee<Fr, Pl, Bearer>(
    mut fee_mismatch: i64,
    rescale_factor: Ratio<u64>,
    mut instructions: Vec<Execution<Fr, Pl, Bearer>>,
) -> Vec<Execution<Fr, Pl, Bearer>>
where
    Fr: MarketTaker + TakerBehaviour + Copy,
{
    for i in &mut instructions {
        if let Either::Left(take) = i {
            let delta = take.scale_consumed_budget(rescale_factor);
            fee_mismatch += delta;
        }
    }
    for i in &mut instructions {
        if let Either::Left(take) = i {
            if fee_mismatch != 0 {
                let delta = take.correct_consumed_budget(-fee_mismatch);
                fee_mismatch += delta;
            } else {
                break;
            }
        }
    }
    instructions
}

fn execute<Tk, Mk, Ctx>(
    mut ctx: Ctx,
    mut state: ExecutionState,
    mut effects: Vec<EffectPreview<Either<Tk, Mk>>>,
    instructions: Vec<Execution<Tk, Mk, FinalizedTxOut>>,
) -> (ExecutionState, Vec<EffectPreview<Either<Tk, Mk>>>, Ctx)
where
    Tk: Copy,
    Mk: Copy,
    Magnet<Take<Tk, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Tk>, Ctx>,
    Magnet<Make<Mk, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Mk>, Ctx>,
    Ctx: Clone,
{
    for instruction in instructions {
        match instruction {
            Either::Left(take) => {
                let (new_state, result, new_ctx) = Magnet(take).exec(state, ctx);
                effects.push(result.bimap(|u| u.map(Either::Left), |e| e.map(Either::Left)));
                state = new_state;
                ctx = new_ctx;
            }
            Either::Right(make) => {
                let (new_state, result, new_ctx) = Magnet(make).exec(state, ctx);
                effects.push(result.bimap(|u| u.map(Either::Right), |e| e.map(Either::Right)));
                state = new_state;
                ctx = new_ctx;
            }
        }
    }
    (state, effects, ctx)
}

#[cfg(test)]
mod tests {
    use std::cmp::max;
    use std::fmt::{Display, Formatter};

    use either::Either;
    use num_rational::Ratio;

    use bloom_offchain::execution_engine::bundled::Bundled;
    use bloom_offchain::execution_engine::liquidity_book::core::{Next, TerminalTake, Trans, Unit};
    use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
    use bloom_offchain::execution_engine::liquidity_book::side::Side;
    use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
    use bloom_offchain::execution_engine::liquidity_book::types::{
        AbsolutePrice, ExCostUnits, FeeAsset, InputAsset, OutputAsset,
    };

    use crate::execution_engine::interpreter::balance_fee;

    #[test]
    fn fee_overuse_balancing() {
        let t0_0 = SimpleOrderPF::new(0, 250000);
        let t0_1 = SimpleOrderPF::new(0, 0);
        let t1_0 = SimpleOrderPF::new(0, 250000);
        let t1_1 = SimpleOrderPF::new(0, 0);
        let instructions = vec![
            Either::Left(Trans::new(Bundled(t0_0, ()), Next::Succ(t0_1))),
            Either::Left(Trans::new(Bundled(t1_0, ()), Next::Succ(t1_1))),
        ];
        let reserved_fee = 500000;
        let estimated_fee = 456325;
        let rescale_factor = Ratio::new(estimated_fee, reserved_fee);
        let fee_mismatch = reserved_fee as i64 - estimated_fee as i64;
        let balanced_instructions = balance_fee::<_, (), _>(fee_mismatch, rescale_factor, instructions);
        assert_eq!(
            balanced_instructions
                .iter()
                .map(|i| match i {
                    Either::Left(f) => f.consumed_budget(),
                    _ => 0,
                })
                .sum::<u64>(),
            estimated_fee
        )
    }

    #[test]
    fn fee_overuse_balancing_single() {
        let t0_0 = SimpleOrderPF::new(0, 2000000);
        let t0_1 = SimpleOrderPF::new(0, 0);
        let instructions = vec![Either::Left(Trans::new(Bundled(t0_0, ()), Next::Succ(t0_1)))];
        let reserved_fee = 2000000u64;
        let fee_mismatch = 1658040i64;
        let estimated_fee = reserved_fee - fee_mismatch as u64;
        let rescale_factor = Ratio::new(estimated_fee, reserved_fee);
        let balanced_instructions = balance_fee::<_, (), _>(fee_mismatch, rescale_factor, instructions);
        dbg!(balanced_instructions.clone());
        assert_eq!(
            balanced_instructions
                .iter()
                .map(|i| match i {
                    Either::Left(f) => f.consumed_budget(),
                    _ => 0,
                })
                .sum::<u64>(),
            estimated_fee
        )
    }

    #[test]
    fn fee_underuse_balancing_even() {
        let t0_0 = SimpleOrderPF::new(0, 250000);
        let t0_1 = SimpleOrderPF::new(0, 100000);
        let t1_0 = SimpleOrderPF::new(0, 250000);
        let t1_1 = SimpleOrderPF::new(0, 100000);
        let instructions = vec![
            Either::Left(Trans::new(Bundled(t0_0, ()), Next::Succ(t0_1))),
            Either::Left(Trans::new(Bundled(t1_0, ()), Next::Succ(t1_1))),
        ];
        let reserved_fee = 300000;
        let estimated_fee = 500000;
        let rescale_factor = Ratio::new(estimated_fee, reserved_fee);
        let fee_mismatch = reserved_fee as i64 - estimated_fee as i64;
        let balanced_instructions = balance_fee::<_, (), _>(fee_mismatch, rescale_factor, instructions);
        assert_eq!(
            balanced_instructions
                .iter()
                .map(|i| match i {
                    Either::Left(f) => f.consumed_budget(),
                    _ => 0,
                })
                .sum::<u64>(),
            estimated_fee
        )
    }

    #[test]
    fn fee_underuse_balancing_uneven() {
        let t0_0 = SimpleOrderPF::new(0, 250000);
        let t0_1 = SimpleOrderPF::new(0, 50000);
        let t1_0 = SimpleOrderPF::new(0, 250000);
        let t1_1 = SimpleOrderPF::new(0, 100000);
        let instructions = vec![
            Either::Left(Trans::new(Bundled(t0_0, ()), Next::Succ(t0_1))),
            Either::Left(Trans::new(Bundled(t1_0, ()), Next::Succ(t1_1))),
        ];
        let reserved_fee = 350000;
        let estimated_fee = 500000;
        let rescale_factor = Ratio::new(estimated_fee, reserved_fee);
        let fee_mismatch = reserved_fee as i64 - estimated_fee as i64;
        let balanced_instructions = balance_fee::<_, (), _>(fee_mismatch, rescale_factor, instructions);
        assert_eq!(
            balanced_instructions
                .iter()
                .map(|i| match i {
                    Either::Left(f) => f.consumed_budget(),
                    _ => 0,
                })
                .sum::<u64>(),
            estimated_fee
        )
    }

    /// Order that supports partial filling.
    #[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
    pub struct SimpleOrderPF {
        pub fee: u64,
        pub ex_budget: u64,
    }

    impl Display for SimpleOrderPF {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.write_str(&*format!("Ord(fee={}, budget={})", self.fee, self.ex_budget))
        }
    }

    impl SimpleOrderPF {
        pub fn new(fee: u64, ex_budget: u64) -> Self {
            Self { fee, ex_budget }
        }
    }

    impl MarketTaker for SimpleOrderPF {
        type U = u64;

        fn side(&self) -> Side {
            Side::Ask
        }

        fn input(&self) -> u64 {
            0
        }

        fn output(&self) -> OutputAsset<u64> {
            0
        }

        fn price(&self) -> AbsolutePrice {
            AbsolutePrice::new_unsafe(1, 1)
        }

        fn marginal_cost_hint(&self) -> ExCostUnits {
            0
        }

        fn time_bounds(&self) -> TimeBounds<u64> {
            TimeBounds::None
        }

        fn operator_fee(&self, input_consumed: InputAsset<u64>) -> FeeAsset<u64> {
            0
        }

        fn min_marginal_output(&self) -> OutputAsset<u64> {
            0
        }

        fn fee(&self) -> FeeAsset<u64> {
            self.fee
        }

        fn budget(&self) -> FeeAsset<u64> {
            self.ex_budget
        }

        fn consumable_budget(&self) -> FeeAsset<u64> {
            0
        }
    }

    impl TakerBehaviour for SimpleOrderPF {
        fn with_updated_time(self, time: u64) -> Next<Self, Unit> {
            Next::Succ(self)
        }

        fn with_applied_trade(
            mut self,
            removed_input: InputAsset<u64>,
            added_output: OutputAsset<u64>,
        ) -> Next<Self, TerminalTake> {
            Next::Succ(self)
        }

        fn with_budget_corrected(mut self, delta: i64) -> (i64, Self) {
            let budget_remainder = self.ex_budget as i64;
            let corrected_remainder = budget_remainder + delta;
            let updated_budget_remainder = max(corrected_remainder, 0);
            let real_delta = updated_budget_remainder - budget_remainder;
            self.ex_budget = updated_budget_remainder as u64;
            (real_delta, self)
        }

        fn with_fee_charged(self, fee: u64) -> Self {
            self
        }

        fn with_output_added(self, added_output: u64) -> Self {
            self
        }

        fn try_terminate(self) -> Next<Self, TerminalTake> {
            Next::Succ(self)
        }
    }
}
