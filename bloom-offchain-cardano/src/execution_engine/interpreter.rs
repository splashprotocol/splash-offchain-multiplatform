use cml_chain::auxdata::{AuxiliaryData, ConwayFormatAuxData, Metadata, TransactionMetadatum};
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionBuilder};
use cml_chain::transaction::TransactionOutput;
use cml_core::serialization::StringEncoding;
use either::Either;
use log::{info, trace};
use num_rational::Ratio;
use std::fmt::{Debug, Display, Formatter};
use tailcall::tailcall;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::funding_effect::FundingIO;
use bloom_offchain::execution_engine::liquidity_book::core::{Execution, ExecutionRecipe, Make, Next, Take};
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

const MAX_FEE_CORRECTION_ATTEMPTS: u8 = 24;

#[derive(Debug)]
/// Outcome of a single fee-correction decision.
///
/// The interpreter always starts from the currently matched recipe and the current
/// residue amount. After comparing `estimated_fee` with the effective fee budget,
/// it chooses one of these follow-up actions.
enum FeeCorrection<Fr, Pl, Bearer> {
    /// Rebuild the transaction from the same recipe.
    ///
    /// This is used when the recipe itself does not need to change:
    /// - positive mismatch means we keep the same recipe and increase residue
    /// - negative mismatch with non-zero residue means we keep the same recipe and refund residue
    RebuildSameRecipe {
        /// Whether the next iteration is still allowed to take residual fee into residue.
        take_residual_fee: bool,
        /// Residue value to use on the next iteration after applying this decision.
        updated_accumulated_residue: Lovelace,
    },
    /// Rebuild the transaction from a fee-rebalanced recipe.
    ///
    /// This is used when residue cannot solve the mismatch and we must change
    /// per-take consumed budgets inside the recipe itself.
    RebuildRebalancedRecipe {
        /// Residual-fee mode to use on the next iteration.
        take_residual_fee: bool,
        /// Residue carried into the next iteration together with the rebalanced recipe.
        accumulated_residue: Lovelace,
        /// Recipe after fee balancing changed one or more take states.
        instructions: Vec<Execution<Fr, Pl, Bearer>>,
    },
    /// The current build already has matching reserved and estimated fee.
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecuteRecipeError {
    NoProgress {
        estimated_fee: Lovelace,
        reserved_tx_fee: Lovelace,
        updated_tx_fee: Lovelace,
        accumulated_residue: Lovelace,
        fee_mismatch: i64,
    },
    RetryLimitExceeded {
        attempt: u8,
        estimated_fee: Lovelace,
        reserved_tx_fee: Lovelace,
        updated_tx_fee: Lovelace,
        accumulated_residue: Lovelace,
        fee_mismatch: i64,
    },
}

impl Display for ExecuteRecipeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecuteRecipeError::NoProgress {
                estimated_fee,
                reserved_tx_fee,
                updated_tx_fee,
                accumulated_residue,
                fee_mismatch,
            } => write!(
                f,
                "fee correction made no progress: estimated_fee={}, reserved_tx_fee={}, updated_tx_fee={}, accumulated_residue={}, fee_mismatch={}",
                estimated_fee, reserved_tx_fee, updated_tx_fee, accumulated_residue, fee_mismatch
            ),
            ExecuteRecipeError::RetryLimitExceeded {
                attempt,
                estimated_fee,
                reserved_tx_fee,
                updated_tx_fee,
                accumulated_residue,
                fee_mismatch,
            } => write!(
                f,
                "fee correction exceeded retry limit at attempt {}: estimated_fee={}, reserved_tx_fee={}, updated_tx_fee={}, accumulated_residue={}, fee_mismatch={}",
                attempt, estimated_fee, reserved_tx_fee, updated_tx_fee, accumulated_residue, fee_mismatch
            ),
        }
    }
}

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
            execute_recipe(funding, self.take_residual_fee, ctx, instructions, 0, 0)
                .unwrap_or_else(|err| panic!("fee correction failed: {}", err));

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
    attempt: u8,
) -> Result<
    (
        TransactionBuilder,
        Vec<EffectPreview<Either<Tk, Mk>>>,
        FundingIO<FinalizedTxOut, TransactionOutput>,
        Ctx,
    ),
    ExecuteRecipeError,
>
where
    Tk: MarketTaker + TakerBehaviour + Copy,
    Mk: Copy,
    Magnet<Take<Tk, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Tk>, Ctx>,
    Magnet<Make<Mk, FinalizedTxOut>>: BatchExec<ExecutionState, EffectPreview<Mk>, Ctx>,
    Ctx: Clone + Sized + Has<Collateral> + Has<NetworkId> + Has<OperatorRewardAddress>,
{
    let state = ExecutionState::new();
    info!(
        "fee correction attempt {}: take_residual_fee={}, accumulated_residue={}",
        attempt, take_residual_fee, accumulated_residue
    );
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
    info!(
        "fee correction attempt {} built tx: reserved_tx_fee={}, updated_tx_fee={}, estimated_fee={}, fee_mismatch={}, operator_interest={}, funding_io={}, recipe_state={}",
        attempt,
        reserved_tx_fee,
        updated_tx_fee,
        estimated_fee,
        fee_mismatch,
        operator_interest,
        funding_io_kind(&funding_io),
        describe_fee_balance_state(&instructions)
    );
    trace!(
        "Est. fee: {}, reserved fee: {}, updated fee: {}, accumulated residue: {}, mismatch: {}, funding io: {}",
        estimated_fee,
        reserved_tx_fee,
        updated_tx_fee,
        accumulated_residue,
        fee_mismatch,
        funding_io_kind(&funding_io)
    );
    if fee_mismatch != 0 && attempt >= MAX_FEE_CORRECTION_ATTEMPTS {
        info!(
            "fee correction attempt {} hit retry limit with mismatch {}",
            attempt, fee_mismatch
        );
        return Err(ExecuteRecipeError::RetryLimitExceeded {
            attempt,
            estimated_fee,
            reserved_tx_fee,
            updated_tx_fee,
            accumulated_residue,
            fee_mismatch,
        });
    }
    let original_state = fee_balance_state(&instructions);
    match decide_fee_correction(
        take_residual_fee,
        fee_mismatch,
        reserved_tx_fee,
        estimated_fee,
        accumulated_residue,
        instructions.clone(),
    ) {
        FeeCorrection::Complete => {
            info!("fee correction converged at attempt {}", attempt);
            Ok((tx_builder, effects, funding_io, ctx))
        }
        FeeCorrection::RebuildSameRecipe {
            take_residual_fee,
            updated_accumulated_residue,
        } => {
            info!(
                "fee correction attempt {} rebuilding same recipe with take_residual_fee={} and accumulated_residue={}",
                attempt,
                take_residual_fee,
                updated_accumulated_residue
            );
            execute_recipe(
                funding,
                take_residual_fee,
                ctx,
                instructions,
                updated_accumulated_residue,
                attempt + 1,
            )
        }
        FeeCorrection::RebuildRebalancedRecipe {
            take_residual_fee,
            accumulated_residue,
            instructions,
        } => {
            let corrected_state = fee_balance_state(&instructions);
            if corrected_state == original_state {
                info!(
                    "fee correction attempt {} made no progress: state stayed {}",
                    attempt,
                    describe_state_entries(&corrected_state)
                );
                Err(ExecuteRecipeError::NoProgress {
                    estimated_fee,
                    reserved_tx_fee,
                    updated_tx_fee,
                    accumulated_residue,
                    fee_mismatch,
                })
            } else {
                info!(
                    "fee correction attempt {} rebalanced recipe state from {} to {}",
                    attempt,
                    describe_state_entries(&original_state),
                    describe_state_entries(&corrected_state)
                );
                execute_recipe(
                    funding,
                    take_residual_fee,
                    ctx,
                    instructions,
                    accumulated_residue,
                    attempt + 1,
                )
            }
        }
    }
}

fn decide_fee_correction<Fr, Pl, Bearer>(
    take_residual_fee: bool,
    fee_mismatch: i64,
    reserved_tx_fee: Lovelace,
    estimated_fee: Lovelace,
    accumulated_residue: Lovelace,
    instructions: Vec<Execution<Fr, Pl, Bearer>>,
) -> FeeCorrection<Fr, Pl, Bearer>
where
    Fr: MarketTaker + TakerBehaviour + Copy,
{
    if fee_mismatch == 0 {
        FeeCorrection::Complete
    } else if take_residual_fee && fee_mismatch > 0 {
        info!(
            "fee correction decision: keep recipe and accumulate residue by {}",
            fee_mismatch.unsigned_abs()
        );
        FeeCorrection::RebuildSameRecipe {
            take_residual_fee: true,
            updated_accumulated_residue: accumulated_residue + fee_mismatch.unsigned_abs(),
        }
    } else if fee_mismatch < 0 && accumulated_residue > 0 {
        let residue_refund = fee_mismatch.unsigned_abs().min(accumulated_residue);
        info!(
            "fee correction decision: refund residue by {} before rebalancing recipe",
            residue_refund
        );
        FeeCorrection::RebuildSameRecipe {
            take_residual_fee,
            updated_accumulated_residue: accumulated_residue - residue_refund,
        }
    } else {
        info!(
            "fee correction decision: rebalance recipe budgets with rescale_factor={}/{}",
            estimated_fee, reserved_tx_fee
        );
        let fee_rescale_factor = Ratio::new(estimated_fee, reserved_tx_fee);
        let corrected_recipe = balance_fee(fee_mismatch, fee_rescale_factor, instructions);
        FeeCorrection::RebuildRebalancedRecipe {
            take_residual_fee: false,
            accumulated_residue,
            instructions: corrected_recipe,
        }
    }
}

fn describe_fee_balance_state<Fr, Pl, Bearer>(instructions: &[Execution<Fr, Pl, Bearer>]) -> String
where
    Fr: MarketTaker,
{
    describe_state_entries(&fee_balance_state(instructions))
}

fn describe_state_entries(state: &[(bool, Lovelace, Lovelace)]) -> String {
    if state.is_empty() {
        return "[]".to_string();
    }
    state
        .iter()
        .enumerate()
        .map(|(ix, (is_terminal, consumed_budget, remaining_budget))| {
            format!(
                "#{}:{} consumed={} remaining={}",
                ix,
                if *is_terminal { "term" } else { "succ" },
                consumed_budget,
                remaining_budget
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn fee_balance_state<Fr, Pl, Bearer>(
    instructions: &[Execution<Fr, Pl, Bearer>],
) -> Vec<(bool, Lovelace, Lovelace)>
where
    Fr: MarketTaker,
{
    instructions
        .iter()
        .filter_map(|instruction| match instruction {
            Either::Left(take) => {
                let (is_terminal, remaining_budget) = match &take.result {
                    Next::Succ(next) => (false, next.budget()),
                    Next::Term(term) => (true, term.remaining_budget),
                };
                Some((is_terminal, take.consumed_budget(), remaining_budget))
            }
            Either::Right(_) => None,
        })
        .collect()
}

fn recipe_fee_balance_progressed<Fr, Pl, Bearer>(
    before: &[Execution<Fr, Pl, Bearer>],
    after: &[Execution<Fr, Pl, Bearer>],
) -> bool
where
    Fr: MarketTaker,
{
    fee_balance_state(before) != fee_balance_state(after)
}

fn funding_io_kind<I, O>(funding_io: &FundingIO<I, O>) -> &'static str {
    match funding_io {
        FundingIO::Added(_, _) => "Added",
        FundingIO::Replaced(_, _) => "Replaced",
        FundingIO::NotUsed(_) => "NotUsed",
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
    use std::collections::HashSet;
    use std::fmt::{Display, Formatter};
    use std::sync::Arc;

    use algebra_core::monoid::Monoid;
    use cml_chain::builders::tx_builder::TransactionUnspentOutput;
    use cml_chain::{address::Address, transaction::TransactionOutput, Value};
    use cml_core::serialization::Deserialize;
    use cml_crypto::{Ed25519KeyHash, TransactionHash};
    use either::Either;
    use num_rational::Ratio;
    use type_equalities::IsEqual;

    use bloom_offchain::execution_engine::bundled::Bundled;
    use bloom_offchain::execution_engine::funding_effect::FundingIO;
    use bloom_offchain::execution_engine::liquidity_book::config::{ExecutionCap, ExecutionConfig};
    use bloom_offchain::execution_engine::liquidity_book::core::ExecutionRecipe;
    use bloom_offchain::execution_engine::liquidity_book::core::{Next, Take, TerminalTake, Trans, Unit};
    use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
    use bloom_offchain::execution_engine::liquidity_book::side::Side;
    use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
    use bloom_offchain::execution_engine::liquidity_book::types::{
        AbsolutePrice, ExCostUnits, FeeAsset, InputAsset, OutputAsset,
    };
    use bloom_offchain::execution_engine::liquidity_book::{ExternalLBEvents, LiquidityBook, TLB};
    use spectrum_cardano_lib::collateral::Collateral;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::output::FinalizedTxOut;
    use spectrum_cardano_lib::protocol_params::constant_tx_builder;
    use spectrum_cardano_lib::transaction::TransactionOutputExtension;
    use spectrum_cardano_lib::{NetworkId, OutputRef, Token};
    use spectrum_offchain::data::small_vec::SmallVec;
    use spectrum_offchain::domain::{Has, Stable, Tradable};
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain_cardano::constants::ADDITIONAL_FEE;
    use spectrum_offchain_cardano::creds::{OperatorCred, OperatorRewardAddress};
    use spectrum_offchain_cardano::data::pair::PairId;
    use spectrum_offchain_cardano::data::pool::{AnyPool, PoolValidation};
    use spectrum_offchain_cardano::deployment::ProtocolValidator::{
        BalanceFnPoolV1, BalanceFnPoolV2, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee,
        ConstFnPoolFeeSwitchV2, ConstFnPoolV1, ConstFnPoolV2, LimitOrderV1, LimitOrderWitnessV1,
        RoyaltyPoolV1, RoyaltyPoolV1LedgerFixed, RoyaltyPoolV2, StableFnPoolT2T,
    };
    use spectrum_offchain_cardano::deployment::{
        DeployedScriptInfo, DeployedValidator, DeployedValidators, ProtocolScriptHashes,
    };
    use spectrum_offchain_cardano::handler_context::{ConsumedIdentifiers, ConsumedInputs};

    use crate::execution_engine::execution_state::ExecutionState;
    use crate::execution_engine::interpreter::{
        balance_fee, decide_fee_correction, execute, fee_balance_state, recipe_fee_balance_progressed,
        FeeCorrection,
    };
    use crate::orders::limit::{LimitOrder, LimitOrderValidation};

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

    #[test]
    fn fee_underuse_balancing_cannot_progress_for_terminal_orders_with_zero_budget_remainder() {
        let t0_0 = SimpleOrderPF::new(0, 671_771);
        let t1_0 = SimpleOrderPF::new(0, 671_771);
        let instructions = vec![
            Either::Left(Trans::new(
                Bundled(t0_0, ()),
                Next::Term(TerminalTake {
                    remaining_input: 0,
                    accumulated_output: 0,
                    remaining_budget: 0,
                    remaining_fee: 0,
                }),
            )),
            Either::Left(Trans::new(
                Bundled(t1_0, ()),
                Next::Term(TerminalTake {
                    remaining_input: 0,
                    accumulated_output: 0,
                    remaining_budget: 0,
                    remaining_fee: 0,
                }),
            )),
        ];
        let reserved_fee = instructions
            .iter()
            .map(|i| match i {
                Either::Left(f) => f.consumed_budget(),
                _ => 0,
            })
            .sum::<u64>();
        let estimated_fee = 1_456_278u64;
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
            reserved_fee
        );
    }

    #[test]
    fn exact_iag_recipe_rebuild_adds_hidden_funding_io_and_increases_fee() {
        const ORDER_TX: &str = "4d906571e8d943a9f6c54970a65fa271a075d1ebb1c5957ea8d0d89acfbb2314";
        const ORDER_IX: u64 = 0;
        const BEACON_INPUT_TX: &str = "b1690e06b88019064b91c97ac717a5b7a6e1a0dfd7f4273d2fe52e4149681624";
        const BEACON_INPUT_IX: u64 = 0;
        const POOL_TX: &str = "909e94bd7eadea617527a2b71acb2d0a103989cc5ca8d905e9d7a874ffaa20f5";
        const POOL_IX: u64 = 0;
        const ORDER_BEACON: &str = "73bfc3a2be32cc9d9e34c6f8f5af2a9c1d0297df6ca93026d503ac0a";
        const ORDER_UTXO: &str = "a300583911464eeee89f05aff787d40045af2a40a83fd96c513197d32fbc54ff0236b3ff1ec77ed8b241273ac7c0c2d5f1ae8776728456c0fbc35c3f7e01821a002625a0a1581c5d16cc1a177b5d9ba9cfa9793b07e60f1fb70fea1f8aef064415d114a1434941471a3adfd3f6028201d81858f2d8798c4100581c08636ad2419cdc4076333a18dbbeddbe25f72ef20e57ea0973c20a3cd87982581c5d16cc1a177b5d9ba9cfa9793b07e60f1fb70fea1f8aef064415d114434941471a3adfd3f61a000f42401a0aba7d87d879824040d879821a0aba7d871a3adfd3f600d87982d87981581c73bfc3a2be32cc9d9e34c6f8f5af2a9c1d0297df6ca93026d503ac0ad87981d87981d87981581c36b3ff1ec77ed8b241273ac7c0c2d5f1ae8776728456c0fbc35c3f7e581c73bfc3a2be32cc9d9e34c6f8f5af2a9c1d0297df6ca93026d503ac0a81581c5cb2c968e5d1c7197a6ce7615967310a375545d9bc65063a964335b2";
        const POOL_UTXO: &str = "a3005839319dee0659686c3ab807895c929e3284c11222affd710b09be690f924db2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821b000000050cca9844a3581c4f7dd6afaba351eca93dce82ecd2d489cb950bce7de3a89b07c37878a14a4941475f4144415f4c511b7ffffff5bc42696e581c5d16cc1a177b5d9ba9cfa9793b07e60f1fb70fea1f8aef064415d114a1434941471b0000001ad141d089581c8475b1a7546a1a8eb929b27868797b0a3ffcfeb547fc1e249cfe13bda14b4941475f4144415f4e465401028201d81858e3d8799fd8799f581c8475b1a7546a1a8eb929b27868797b0a3ffcfeb547fc1e249cfe13bd4b4941475f4144415f4e4654ffd8799f4040ffd8799f581c5d16cc1a177b5d9ba9cfa9793b07e60f1fb70fea1f8aef064415d11443494147ffd8799f581c4f7dd6afaba351eca93dce82ecd2d489cb950bce7de3a89b07c378784a4941475f4144415f4c51ff1a0001843c18441a10ba7aa21a4cd9f6ab9fd8799fd87a9f581c8d5e497bb0507f0ae64b0d9c2f4f3544c9244ed3f72cf16b77706663ffffff00581c75c4570eb625ae881b32a34c52b159f6f3f3f2c7aaabf5bac4688133ff";
        const FUNDING_TX: &str = "556067b45db9b6b3fac38e2e8a173de4374b171b52c7a32a37f991330189e146";
        const FUNDING_IX: u64 = 2;
        const FUNDING_UTXO: &str = "825839015cb2c968e5d1c7197a6ce7615967310a375545d9bc65063a964335b2213c52886a517be9954d80ec7ba19ca783a03eb501694b9281dfcad81a00167496";
        const RESIDUE: u64 = 473_756;

        let scripts = mainnet_scripts();
        let deployment = Arc::new(mainnet_deployment());
        let order_ref = OutputRef::new(TransactionHash::from_hex(ORDER_TX).unwrap(), ORDER_IX);
        let beacon_input_ref = OutputRef::new(
            TransactionHash::from_hex(BEACON_INPUT_TX).unwrap(),
            BEACON_INPUT_IX,
        );
        let pool_ref = OutputRef::new(TransactionHash::from_hex(POOL_TX).unwrap(), POOL_IX);
        let funding_ref = OutputRef::new(TransactionHash::from_hex(FUNDING_TX).unwrap(), FUNDING_IX);

        let order_bearer = TransactionOutput::from_cbor_bytes(&hex::decode(ORDER_UTXO).unwrap()).unwrap();
        let pool_bearer = TransactionOutput::from_cbor_bytes(&hex::decode(POOL_UTXO).unwrap()).unwrap();
        let funding_bearer = TransactionOutput::from_cbor_bytes(&hex::decode(FUNDING_UTXO).unwrap()).unwrap();

        let collateral = Collateral::from(TransactionUnspentOutput::new(
            OutputRef::new(
                TransactionHash::from_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                    .unwrap(),
                0,
            )
            .into(),
            TransactionOutput::new(
                funding_bearer.address().clone(),
                Value::from(5_000_000),
                None,
                None,
            ),
        ));
        let mock_reference_utxo = TransactionUnspentOutput::new(
            OutputRef::new(
                TransactionHash::from_hex("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
                    .unwrap(),
                0,
            )
            .into(),
            TransactionOutput::new(
                funding_bearer.address().clone(),
                Value::from(2_000_000),
                None,
                None,
            ),
        );

        let ctx = ExactLedgerContext {
            scripts,
            deployment,
            oref: order_ref,
            consumed_inputs: SmallVec::new(vec![beacon_input_ref].into_iter()).into(),
            consumed_identifiers: Default::default(),
            cred: OperatorCred(
                Ed25519KeyHash::from_hex("5cb2c968e5d1c7197a6ce7615967310a375545d9bc65063a964335b2").unwrap(),
            ),
            limit_validation: LimitOrderValidation {
                min_cost_per_ex_step: 600_000,
                min_fee_lovelace: 0,
            },
            pool_validation: PoolValidation {
                min_n2t_lovelace: 150_000_000,
                min_t2t_lovelace: 10_000_000,
            },
            network_id: NetworkId::MAINNET,
            operator_reward: OperatorRewardAddress(funding_bearer.address().clone()),
            collateral,
            mock_reference_utxo,
        };

        let order =
            LimitOrder::try_from_ledger(&order_bearer, &ctx).expect("failed to parse exact IAG order");
        let pool = AnyPool::try_from_ledger(&pool_bearer, &ctx).expect("failed to parse exact IAG pool");

        let mut book = TLB::<LimitOrder, AnyPool, PairId, ExUnits>::new(
            0,
            ExecutionConfig {
                execution_cap: ExecutionCap {
                    soft: ExUnits {
                        mem: 5_000_000,
                        steps: 4_000_000_000,
                    },
                    hard: ExUnits {
                        mem: 14_000_000,
                        steps: 10_000_000_000,
                    },
                },
                o2o_allowed: true,
                base_step_budget: 600_000.into(),
            },
            order.pair_id(),
        );
        book.update_taker(order);
        book.update_maker(pool);

        let (recipe, _) = book.attempt();
        let recipe = recipe.expect("expected exact IAG order/pool recipe");
        let (ExecutionRecipe(instructions), linked) = ExecutionRecipe::link(recipe, |id| {
            if id == order.stable_id() {
                Some((order_ref, FinalizedTxOut(order_bearer.clone(), order_ref)))
            } else if id == pool.stable_id() {
                Some((pool_ref, FinalizedTxOut(pool_bearer.clone(), pool_ref)))
            } else {
                None
            }
        })
        .expect("expected bearers to link");
        assert_eq!(linked, HashSet::from([order_ref, pool_ref]));

        let (
            ExecutionState {
                tx_blueprint,
                reserved_tx_fee,
                operator_interest,
            },
            _effects,
            _,
        ) = execute(
            ctx.clone(),
            ExecutionState::new(),
            Vec::new(),
            instructions.clone(),
        );

        assert_eq!(reserved_tx_fee, 1_000_000);
        assert_eq!(operator_interest, 0);
        assert_eq!(tx_blueprint.script_io.len(), 2);

        let (mut builder_without_residue, funding_io_without_residue) = execute(
            ctx.clone(),
            ExecutionState::new(),
            Vec::new(),
            instructions.clone(),
        )
        .0
        .tx_blueprint
        .project_onto_builder(
            constant_tx_builder(),
            ctx.network_id,
            ctx.operator_reward.clone(),
            FinalizedTxOut(funding_bearer.clone(), funding_ref),
            operator_interest,
        );
        builder_without_residue
            .add_collateral(ctx.collateral.clone().into())
            .unwrap();
        let fee_without_residue = builder_without_residue.min_fee(true).unwrap() + ADDITIONAL_FEE;

        let (mut builder_with_residue, funding_io_with_residue) =
            execute(ctx.clone(), ExecutionState::new(), Vec::new(), instructions)
                .0
                .tx_blueprint
                .project_onto_builder(
                    constant_tx_builder(),
                    ctx.network_id,
                    ctx.operator_reward.clone(),
                    FinalizedTxOut(funding_bearer.clone(), funding_ref),
                    operator_interest + RESIDUE,
                );
        builder_with_residue
            .add_collateral(ctx.collateral.clone().into())
            .unwrap();
        let fee_with_residue = builder_with_residue.min_fee(true).unwrap() + ADDITIONAL_FEE;

        assert!(matches!(funding_io_without_residue, FundingIO::NotUsed(_)));
        match funding_io_with_residue {
            FundingIO::Replaced(FinalizedTxOut(input, input_ref), output) => {
                assert_eq!(input_ref, funding_ref);
                assert_eq!(input.value().coin, 1_471_638);
                assert_eq!(output.value().coin, 1_471_638 + RESIDUE);
            }
            _ => panic!("expected funding replacement"),
        }

        assert_eq!(builder_without_residue.get_inputs().len(), 2);
        assert_eq!(builder_without_residue.get_outputs().len(), 2);
        assert_eq!(builder_with_residue.get_inputs().len(), 3);
        assert_eq!(builder_with_residue.get_outputs().len(), 3);
        assert_eq!(fee_without_residue, 526_244);
        assert_eq!(fee_with_residue, 531_914);
    }

    #[test]
    fn exact_iag_negative_mismatch_is_resolved_by_refunding_residue() {
        const RESERVED_TX_FEE: u64 = 1_000_000;
        const INITIAL_RESIDUE: u64 = 473_756;
        const REBUILT_ESTIMATED_FEE: u64 = 531_914;

        let correction = decide_fee_correction::<SimpleOrderPF, Unit, ()>(
            true,
            (RESERVED_TX_FEE - INITIAL_RESIDUE) as i64 - REBUILT_ESTIMATED_FEE as i64,
            RESERVED_TX_FEE,
            REBUILT_ESTIMATED_FEE,
            INITIAL_RESIDUE,
            Vec::new(),
        );

        match correction {
            FeeCorrection::RebuildSameRecipe {
                take_residual_fee,
                updated_accumulated_residue,
            } => {
                assert!(take_residual_fee);
                assert_eq!(updated_accumulated_residue, 468_086);
                assert_eq!(
                    RESERVED_TX_FEE - updated_accumulated_residue,
                    REBUILT_ESTIMATED_FEE
                );
            }
            _ => panic!("expected residue rollback to close the mismatch"),
        }
    }

    #[test]
    fn negative_mismatch_consumes_accumulated_residue_before_rebalancing_recipe() {
        let correction = decide_fee_correction::<SimpleOrderPF, Unit, ()>(
            true,
            -5_670,
            1_000_000,
            526_244,
            473_756,
            Vec::new(),
        );

        match correction {
            FeeCorrection::RebuildSameRecipe {
                take_residual_fee,
                updated_accumulated_residue,
            } => {
                assert!(take_residual_fee);
                assert_eq!(updated_accumulated_residue, 468_086);
            }
            _ => panic!("expected residue rollback before recipe rebalance"),
        }
    }

    #[test]
    fn negative_mismatch_without_residue_rebalances_recipe() {
        let instructions = vec![Either::Left(Trans::new(
            Bundled(SimpleOrderPF::new(0, 1_000_000), ()),
            Next::Succ(SimpleOrderPF::new(0, 50_000)),
        ))];
        let original_consumed_budget = match &instructions[0] {
            Either::Left(take) => take.consumed_budget(),
            Either::Right(_) => unreachable!("expected take instruction"),
        };

        let correction = decide_fee_correction::<SimpleOrderPF, Unit, ()>(
            false,
            -10_000,
            1_000_000,
            1_010_000,
            0,
            instructions,
        );

        match correction {
            FeeCorrection::RebuildRebalancedRecipe {
                take_residual_fee,
                accumulated_residue,
                instructions,
            } => {
                assert!(!take_residual_fee);
                assert_eq!(accumulated_residue, 0);
                assert_eq!(instructions.len(), 1);
                let take = match &instructions[0] {
                    Either::Left(take) => take,
                    Either::Right(_) => panic!("expected take instruction"),
                };
                assert!(take.consumed_budget() > original_consumed_budget);
            }
            _ => panic!("expected recipe rebalance when there is no residue to refund"),
        }
    }

    #[test]
    fn unchanged_terminal_recipe_is_detected_as_no_progress() {
        let take = Take {
            target: Bundled(
                SimpleOrderPF {
                    fee: 0,
                    ex_budget: 1_000_000,
                },
                (),
            ),
            result: Next::Term(TerminalTake {
                remaining_input: 0,
                accumulated_output: 0,
                remaining_budget: 0,
                remaining_fee: 0,
            }),
        };
        let instructions = vec![Either::Left(take)];

        let corrected = balance_fee::<SimpleOrderPF, Unit, ()>(
            -56_368,
            Ratio::new(1_056_368, 1_000_000),
            instructions.clone(),
        );

        assert_eq!(fee_balance_state(&instructions), fee_balance_state(&corrected),);
        assert!(!recipe_fee_balance_progressed(&instructions, &corrected));
    }

    fn mainnet_scripts() -> ProtocolScriptHashes {
        let path = format!(
            "{}/../bloom-cardano-agent/resources/mainnet.deployment.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let raw_deployment = std::fs::read_to_string(path).expect("Cannot load deployment file");
        let deployment: DeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        ProtocolScriptHashes::from(&deployment)
    }

    fn mainnet_deployment() -> DeployedValidators {
        let path = format!(
            "{}/../bloom-cardano-agent/resources/mainnet.deployment.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let raw_deployment = std::fs::read_to_string(path).expect("Cannot load deployment file");
        serde_json::from_str(&raw_deployment).expect("Invalid deployment file")
    }

    struct ExactLedgerContext {
        scripts: ProtocolScriptHashes,
        deployment: Arc<DeployedValidators>,
        oref: OutputRef,
        consumed_inputs: ConsumedInputs,
        consumed_identifiers: ConsumedIdentifiers<Token>,
        cred: OperatorCred,
        limit_validation: LimitOrderValidation,
        pool_validation: PoolValidation,
        network_id: NetworkId,
        operator_reward: OperatorRewardAddress,
        collateral: Collateral,
        mock_reference_utxo: TransactionUnspentOutput,
    }

    impl Clone for ExactLedgerContext {
        fn clone(&self) -> Self {
            Self {
                scripts: self.scripts,
                deployment: Arc::clone(&self.deployment),
                oref: self.oref,
                consumed_inputs: self.consumed_inputs,
                consumed_identifiers: self.consumed_identifiers,
                cred: self.cred,
                limit_validation: self.limit_validation,
                pool_validation: self.pool_validation,
                network_id: self.network_id,
                operator_reward: self.operator_reward.clone(),
                collateral: self.collateral.clone(),
                mock_reference_utxo: self.mock_reference_utxo.clone(),
            }
        }
    }

    fn dummy_funding_output() -> TransactionOutput {
        TransactionOutput::new(
            Address::from_bech32(
                "addr1z8d70g7c58vznyye9guwagdza74x36f3uff0eyk2zwpcpx6c96rgsm7p0hmwrj8e28qny5yxwya63e8gjj8s2ugfglhsxedx9j",
            )
            .unwrap(),
            Value::from(2_000_000),
            None,
            None,
        )
    }

    fn dummy_collateral(funding_bearer: &TransactionOutput) -> Collateral {
        Collateral::from(TransactionUnspentOutput::new(
            OutputRef::new(
                TransactionHash::from_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                    .unwrap(),
                0,
            )
            .into(),
            TransactionOutput::new(
                funding_bearer.address().clone(),
                Value::from(5_000_000),
                None,
                None,
            ),
        ))
    }

    fn dummy_reference_utxo(funding_bearer: &TransactionOutput) -> TransactionUnspentOutput {
        TransactionUnspentOutput::new(
            OutputRef::new(
                TransactionHash::from_hex("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
                    .unwrap(),
                0,
            )
            .into(),
            TransactionOutput::new(
                funding_bearer.address().clone(),
                Value::from(2_000_000),
                None,
                None,
            ),
        )
    }

    fn actual_limit_order_reference_utxo() -> TransactionUnspentOutput {
        reference_utxo_from_cbor(
            "b91eda29d145ab6c0bc0d6b7093cb24b131440b7b015033205476f39c690a51f",
            0,
            "a300581d714ec50f2624ba62043bee44448bc9b388fa172e0b73ef94c3ab2f809401821a00547158a003d818590435820259043059042d01000033232323232323222323232232253330093232533300b0041323300100137566022602460246024602460246024601c6ea8008894ccc040004528099299980719baf00d300f301300214a226600600600260260022646464a66601c6014601e6ea80044c94ccc03cc030c040dd5000899191929998090038a99980900108008a5014a066ebcc020c04cdd5001180b180b980b980b980b980b980b980b980b980b98099baa00f3375e600860246ea8c010c048dd5180a98091baa00230043012375400260286eb0c050c054c054c044dd50028b1991191980080080191299980a8008a60103d87a80001323253330143375e6016602c6ea80080144cdd2a40006603000497ae0133004004001301900230170013758600a60206ea8010c04cc040dd50008b180098079baa0052301230130013322323300100100322533301200114a0264a66602066e3cdd7180a8010020a5113300300300130150013758602060226022602260226022602260226022601a6ea8004dd71808180898089808980898089808980898089808980898069baa0093001300c37540044601e00229309b2b19299980598050008a999804180218048008a51153330083005300900114a02c2c6ea8004c8c94ccc01cc010c020dd50028991919191919191919191919191919191919191919191919299981118128010991919191924c646600200200c44a6660500022930991980180198160011bae302a0015333022301f30233754010264646464a666052605800426464931929998141812800899192999816981800109924c64a666056605000226464a66606060660042649318140008b181880098169baa0021533302b3027001132323232323253330343037002149858dd6981a800981a8011bad30330013033002375a6062002605a6ea800858c0acdd50008b181700098151baa0031533302830240011533302b302a37540062930b0b18141baa002302100316302a001302a0023028001302437540102ca666042603c60446ea802c4c8c8c8c94ccc0a0c0ac00852616375a605200260520046eb4c09c004c08cdd50058b180d006180c8098b1bac30230013023002375c60420026042004603e002603e0046eb4c074004c074008c06c004c06c008c064004c064008dd6980b800980b8011bad30150013015002375a60260026026004602200260220046eb8c03c004c03c008dd7180680098049baa0051625333007300430083754002264646464a66601c60220042930b1bae300f001300f002375c601a00260126ea8004588c94ccc01cc0100044c8c94ccc030c03c00852616375c601a00260126ea800854ccc01cc00c0044c8c94ccc030c03c00852616375c601a00260126ea800858c01cdd50009b8748008dc3a4000ae6955ceaab9e5573eae815d0aba24c0126d8799fd87a9f581c96f5c1bee23481335ff4aece32fe1dfa1aa40a944a66d2d6edc9a9a5ffff0001",
        )
    }

    fn actual_const_fn_pool_fee_switch_v2_reference_utxo() -> TransactionUnspentOutput {
        reference_utxo_from_cbor(
            "482719d050a96b91206f78c6c2b834e844ea576557387e09e7b1c8afa4d433ac",
            0,
            "a300581d714ec50f2624ba62043bee44448bc9b388fa172e0b73ef94c3ab2f809401821a02df8520a003d818592ae18202592adc592ad901000032323232323232323232323232323232323232323232323232323232323232323232323232323232222533302732323232323232323253330303370e900100109919192998129980a801981a8038a99981999b8753330333370e6eb4c0d4031200014800054ccc0cccdc39bad303500c4800852002153330333370e6eb4c0d4031200414801054ccc0cccdc39bad303500c480185200614802120001323232323232323232323232323232323232323253330473370e90020010991919191929981f19b8700b00c1533304c3370ea66609866e1cdd69827012a4000290000a99982619b87375a609c04a90010a40042a66609866e1cdd69827012a4008290020a99982619b87375a609c04a90030a400c29004240002646464a66609e66e1d20040021325330423370e6609e44a66609400229000099b803033375660ac60a8002600460a60020106609e44a66609400229000099b803033375660ac60a8002600460a600200e2a6608466ebc00402454cc108cc0c801401854cc108cdc499b823370202203c02a66e08cdc080a00a80f099b893370466e0404407804ccdc119b8101201301e3051001163052002304b00137540142a66609866e1d4ccc130cdc39bad304e02548000520001533304c3370e6eb4c138095200214800854ccc130cdc39bad304e02548010520041533304c3370e6eb4c138095200614801852008480084c8c8c94ccc13ccdc3a4008004264a6608466e1ccc13c894ccc1280045200013370060666eacc158c150004c008c14c004020cc13c894ccc1280045200013370060666eacc158c150004c008c14c00401c54cc108cdd78008048a99821198190028030a9982119b893370466e04044078054cdc119b8101401501e13371266e08cdc080880f00999b823370202402603c60a20022c60a400460960026ea802854ccc130cdc3a99982619b87375a609c04a90000a40002a66609866e1cdd69827012a4004290010a99982619b87375a609c04a90020a40082a66609866e1cdd69827012a400c290030a40109002099191929982099b873304e2253330490011480004cdc018191bab30553053001300230520010073304e2253330490011480004cdc018191bab305530530013002305200100615330413371200266e08051200415330413303100400515330413370e66e0404007520001533041533304f33710900019b81013014133712605866e08cdc080880919b803370402805a66e08cdc080980a19b810030023370402466e08cdc080980a19b81003002133712605866e08cdc080980a19b803370402405a66e08cdc080880919b810030023370402866e08cdc080880919b810030021333222323232323232323232323253304f3303f3374a90001982b982f8059982b982f8051982b982f8049982b982f8041982b9ba8375a60be00e660ae6ea0dd6982f8031982b9ba8375a60be002660ae6ea0dd6982f982f0009982b9ba7375860be006660ae6ea0dd6982f8011982b9ba9375c60be60bc0040b201c2a6609ea6609e66e24cdc101da99982e99b88480000344cdc09bad305f001375a60be00a266e04dd6982f982f0009bad305f004533305d337109000006899b8200d375a60be00c266e08030dd6982f803099b88533305d337109000006899b8200d375a60be00c266e08030dd6982f80319b8203b33700a6660ba66e21200000d1337026eb4c17c004dd6982f802899b81375a60be60bc0026eb4c17c01120021533305d337109000006899b87375a60be0086eb4c17cc1780044cdc39bad305f005375a60be002608a60be01a60b800260b600260b400260b200260b000260ae00260ac00260aa00260a800260aa05c64646464018a6660a466e1d200000213232323232323232323232323232323232323232323232323232323232323232323232323232323232323232533307e3370e6e340052038132324994ccc1dc00452616307f00316375c00260fc00260f80066eb4004c1ec004c1e400ccc1588c8c8c8c80154ccc1eccdc3a400000426464646464646493299983c8008a4c2c61020200ca6660fe66e1d2000002132325333081013370e6e340052038132324994ccc1e80045261630820100316375c0026102020022a6660fe66e1d2002002132325333081013370e6e340052038132324994ccc1e80045261630820100316375c0026102020022c61040200460f60026ea8004c1f400454ccc1eccdc3a400400426464646464646464646493299983e0008a4c2c6108020066eb4004c20c04004c2040400cdd6800984000800983f0019bad001307d00116307e002307700137540026eb0004c1e0004c1d800cdd6800983a80098398019bad00130720013070003375a00260de00260da0066eb4004c1b0004c1a80194ccc1a0cdc3a40000042646464a6660d6a660b666e1c005200013370e002901c0991919299983719b89371a00290200991924ca6660ce0022930b18378018b1bae001306e001306c00416371a0026eb8004c1a800458c1ac008c190004dd500098330009832003299983119b87480000084c8c8c94ccc1954cc154cdc3800a4000266e1c005203813232325333068337126e340052040132324994ccc18400452616306900316375c00260d000260cc0082c6e34004dd700098320008b1832801182f0009baa0013060001305e006533305c3370e90000010991919299982fa9982799b87001480004cdc3800a40702646464a6660c466e24dc6800a40802646493299982d8008a4c2c60c60062c6eb8004c188004c18001058dc68009bae001305e00116305f0023058001375400260b400260b000ca6660ac66e1d2000002132323253330595330493370e0029000099b87001480e04c8c8c94ccc170cdc49b8d001481004c8c9265333055001149858c17400c58dd7000982e000982d0020b1b8d001375c00260b00022c60b200460a40026ea8004c15000458c154008c138004dd500419b81013014337020220246eb4c140c0b8c1440a8dd69827981b98280149bad304e3037304f0281533304c3370ea66609866e1cdd69827012a4000290000a99982619b87375a609c04a90010a40042a66609866e1cdd69827012a4008290020a99982619b87375a609c04a90030a400c290042400c2646464a66609e66e1d20040021325330423370e6609e44a66609400229000099b803033375660ac60a8002600460a60020106609e44a66609400229000099b803033375660ac60a8002600460a600200e2a6608466ebc00402454cc108cc0c801401854cc108cdc499b823370202203c02a66e08cdc080a00a80f099b893370466e0404407804ccdc119b8101201301e3051001163052002304b0013754014264646460646606660a20020046eb0c140c0d4c1440a8dd59827981b18280009827012182680a98260091bab304b0123756609401e60920022c609400460860026ea8004c114c110030c110c10c024dd69821981698220039bad3042302c304300f375a60826078608400a6eb4c100c0fcc104010dd6981f981f18200061bad303e303f002375a607a607c0146605002c00a60740026076607460720186070002607200266044002004606c606e0206eb0c0d4c0d0c0d002854ccc0cccdc3a99981999b87375a606a01890000a40002a66606666e1cdd6981a80624004290010a99981999b87375a606a01890020a40082a66606666e1cdd6981a8062400c290030a4010900109919191919191919191919191919191919191919299982399b87480100084c8c8c8c8c94cc0f8cdc38058060a99982619b87533304c3370e6eb4c138095200014800054ccc130cdc39bad304e02548008520021533304c3370e6eb4c138095200414801054ccc130cdc39bad304e025480185200614802120001323232533304f3370e900200109929982119b873304f22533304a0011480004cdc018199bab30563054001300230530010083304f22533304a0011480004cdc018199bab305630540013002305300100715330423375e0020122a660846606400a00c2a6608466e24cdc119b8101101e0153370466e040500540784cdc499b823370202203c02666e08cdc080900980f18288008b182900118258009baa00a1533304c3370ea66609866e1cdd69827012a4000290000a99982619b87375a609c04a90010a40042a66609866e1cdd69827012a4008290020a99982619b87375a609c04a90030a400c29004240042646464a66609e66e1d20040021325330423370e6609e44a66609400229000099b803033375660ac60a8002600460a60020106609e44a66609400229000099b803033375660ac60a8002600460a600200e2a6608466ebc00402454cc108cc0c801401854cc108cdc499b823370202203c02a66e08cdc080a00a80f099b893370466e0404407804ccdc119b8101201301e3051001163052002304b00137540142a66609866e1d4ccc130cdc39bad304e02548000520001533304c3370e6eb4c138095200214800854ccc130cdc39bad304e02548010520041533304c3370e6eb4c138095200614801852008480104c8c8c94cc104cdc3998271129998248008a4000266e00c0c8dd5982a982980098011829000803998271129998248008a4000266e00c0c8dd5982a9829800980118290008030a9982099b890013370402890020a99820998188020028a9982099b873370202003a90000a99820a99982799b8848000cdc080980a099b89302c3370466e04044048cdc019b8201402d3370466e0404c050cdc080180119b820123370466e0404c050cdc0801801099b89302c3370466e0404c050cdc019b8201202d3370466e04044048cdc080180119b820143370466e04044048cdc0801801099991119191919191919191919192998279981f99ba548000cc15cc17c02ccc15cc17c028cc15cc17c024cc15cc17c020cc15cdd41bad305f0073305737506eb4c17c018cc15cdd41bad305f0013305737506eb4c17cc178004cc15cdd39bac305f0033305737506eb4c17c008cc15cdd49bae305f305e00205900e153304f53304f3371266e080ed4ccc174cdc42400001a266e04dd6982f8009bad305f0051337026eb4c17cc178004dd6982f802299982e99b88480000344cdc10069bad305f0061337040186eb4c17c0184cdc4299982e99b88480000344cdc10069bad305f0061337040186eb4c17c018cdc101d99b80533305d337109000006899b81375a60be0026eb4c17c0144cdc09bad305f305e001375a60be00890010a99982e99b88480000344cdc39bad305f004375a60be60bc002266e1cdd6982f8029bad305f0013045305f00d305c001305b001305a001305900130580013057001305600130550013054001305502e3232323200c53330523370e900000109919191919191919191919191919191919191919191919191919191919191919191919191919191919191919299983f19b87371a002901c0991924ca6660ee0022930b183f8018b1bae001307e001307c003375a00260f600260f2006660ac46464646400aa6660f666e1d20000021323232323232324994ccc1e400452616308101006533307f3370e90000010991929998408099b87371a002901c0991924ca6660f40022930b1841008018b1bae0013081010011533307f3370e90010010991929998408099b87371a002901c0991924ca6660f40022930b1841008018b1bae00130810100116308201002307b001375400260fa0022a6660f666e1d20020021323232323232323232324994ccc1f000452616308401003375a0026106020026102020066eb4004c20004004c1f800cdd6800983e8008b183f001183b8009baa001375800260f000260ec0066eb4004c1d4004c1cc00cdd6800983900098380019bad001306f001306d003375a00260d800260d400ca6660d066e1d20000021323232533306b53305b3370e0029000099b87001480e04c8c8c94ccc1b8cdc49b8d001481004c8c9265333067001149858c1bc00c58dd7000983700098360020b1b8d001375c00260d40022c60d600460c80026ea8004c198004c1900194ccc188cdc3a40000042646464a6660caa660aa66e1c005200013370e002901c0991919299983419b89371a00290200991924ca6660c20022930b18348018b1bae0013068001306600416371a0026eb8004c19000458c194008c178004dd50009830000982f003299982e19b87480000084c8c8c94ccc17d4cc13ccdc3800a4000266e1c005203813232325333062337126e340052040132324994ccc16c00452616306300316375c00260c400260c00082c6e34004dd7000982f0008b182f801182c0009baa001305a001305800653330563370e90000010991919299982ca9982499b87001480004cdc3800a40702646464a6660b866e24dc6800a40802646493299982a8008a4c2c60ba0062c6eb8004c170004c16801058dc68009bae00130580011630590023052001375400260a80022c60aa004609c0026ea8020cdc080980a19b81011012375a60a0605c60a20546eb4c13cc0dcc1400a4dd69827181b98278140a99982619b87533304c3370e6eb4c138095200014800054ccc130cdc39bad304e02548008520021533304c3370e6eb4c138095200414801054ccc130cdc39bad304e025480185200614802120061323232533304f3370e900200109929982119b873304f22533304a0011480004cdc018199bab30563054001300230530010083304f22533304a0011480004cdc018199bab305630540013002305300100715330423375e0020122a660846606400a00c2a6608466e24cdc119b8101101e0153370466e040500540784cdc499b823370202203c02666e08cdc080900980f18288008b182900118258009baa00a13232323032330333051001002375860a0606a60a20546eacc13cc0d8c140004c138090c134054c130048dd598258091bab304a00f304900116304a00230430013754002608a6088018608860860126eb4c10cc0b4c11001cdd69821181618218079bad3041303c3042005375a6080607e60820086eb4c0fcc0f8c100030dd6981f181f8011bad303d303e00a33028016005303a001303b303a303900c3038001303900133022001002303630370103758606a606860680142a66606666e1d4ccc0cccdc39bad303500c4800052000153330333370e6eb4c0d4031200214800854ccc0cccdc39bad303500c4801052004153330333370e6eb4c0d4031200614801852008480104c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c94ccc11ccdc3a400800426464646464a6607c66e1c02c03054ccc130cdc3a99982619b87375a609c04a90000a40002a66609866e1cdd69827012a4004290010a99982619b87375a609c04a90020a40082a66609866e1cdd69827012a400c290030a401090000991919299982799b87480100084c94cc108cdc3998279129998250008a4000266e00c0ccdd5982b182a00098011829800804198279129998250008a4000266e00c0ccdd5982b182a000980118298008038a9982119baf00100915330423303200500615330423371266e08cdc080880f00a99b823370202802a03c266e24cdc119b8101101e0133370466e0404804c078c14400458c148008c12c004dd50050a99982619b87533304c3370e6eb4c138095200014800054ccc130cdc39bad304e02548008520021533304c3370e6eb4c138095200414801054ccc130cdc39bad304e025480185200614802120021323232533304f3370e900200109929982119b873304f22533304a0011480004cdc018199bab30563054001300230530010083304f22533304a0011480004cdc018199bab305630540013002305300100715330423375e0020122a660846606400a00c2a6608466e24cdc119b8101101e0153370466e040500540784cdc499b823370202203c02666e08cdc080900980f18288008b182900118258009baa00a1533304c3370ea66609866e1cdd69827012a4000290000a99982619b87375a609c04a90010a40042a66609866e1cdd69827012a4008290020a99982619b87375a609c04a90030a400c29004240082646464a6608266e1ccc138894ccc1240045200013370060646eacc154c14c004c008c14800401ccc138894ccc1240045200013370060646eacc154c14c004c008c14800401854cc104cdc480099b820144801054cc104cc0c401001454cc104cdc399b8101001d4800054cc1054ccc13ccdc42400066e0404c0504cdc4981619b823370202202466e00cdc100a01699b823370202602866e0400c008cdc100919b823370202602866e0400c0084cdc4981619b823370202602866e00cdc100901699b823370202202466e0400c008cdc100a19b823370202202466e0400c0084ccc888c8c8c8c8c8c8c8c8c8c8c94cc13ccc0fccdd2a4000660ae60be016660ae60be014660ae60be012660ae60be010660ae6ea0dd6982f8039982b9ba8375a60be00c660ae6ea0dd6982f8009982b9ba8375a60be60bc002660ae6e9cdd6182f8019982b9ba8375a60be004660ae6ea4dd7182f982f00102c8070a99827a9982799b8933704076a6660ba66e21200000d1337026eb4c17c004dd6982f802899b81375a60be60bc0026eb4c17c0114ccc174cdc42400001a266e08034dd6982f803099b8200c375a60be00c266e214ccc174cdc42400001a266e08034dd6982f803099b8200c375a60be00c66e080eccdc0299982e99b88480000344cdc09bad305f001375a60be00a266e04dd6982f982f0009bad305f0044800854ccc174cdc42400001a266e1cdd6982f8021bad305f305e00113370e6eb4c17c014dd6982f8009822982f806982e000982d800982d000982c800982c000982b800982b000982a800982a000982a81719191919006299982919b87480000084c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c94ccc1f8cdc39b8d001480e04c8c9265333077001149858c1fc00c58dd7000983f000983e0019bad001307b001307900333056232323232005533307b3370e900000109919191919191924ca6660f20022930b184080803299983f99b87480000084c8c94ccc20404cdc39b8d001480e04c8c926533307a001149858c2080400c58dd70009840808008a99983f99b87480080084c8c94ccc20404cdc39b8d001480e04c8c926533307a001149858c2080400c58dd70009840808008b184100801183d8009baa001307d0011533307b3370e900100109919191919191919191924ca6660f80022930b1842008019bad001308301001308101003375a00261000200260fc0066eb4004c1f400458c1f8008c1dc004dd50009bac00130780013076003375a00260ea00260e60066eb4004c1c8004c1c000cdd6800983780098368019bad001306c001306a00653330683370e900000109919192999835a9982d99b87001480004cdc3800a40702646464a6660dc66e24dc6800a4080264649329998338008a4c2c60de0062c6eb8004c1b8004c1b001058dc68009bae001306a00116306b0023064001375400260cc00260c800ca6660c466e1d2000002132323253330655330553370e0029000099b87001480e04c8c8c94ccc1a0cdc49b8d001481004c8c9265333061001149858c1a400c58dd7000983400098330020b1b8d001375c00260c80022c60ca00460bc0026ea8004c180004c1780194ccc170cdc3a40000042646464a6660bea6609e66e1c005200013370e002901c0991919299983119b89371a00290200991924ca6660b60022930b18318018b1bae0013062001306000416371a0026eb8004c17800458c17c008c160004dd5000982d000982c003299982b19b87480000084c8c8c94ccc1654cc124cdc3800a4000266e1c00520381323232533305c337126e340052040132324994ccc15400452616305d00316375c00260b800260b40082c6e34004dd7000982c0008b182c80118290009baa0013054001163055002304e001375401066e0404c050cdc08088091bad3050302e305102a375a609e606e60a00526eb4c138c0dcc13c0a054ccc130cdc3a99982619b87375a609c04a90000a40002a66609866e1cdd69827012a4004290010a99982619b87375a609c04a90020a40082a66609866e1cdd69827012a400c290030a401090030991919299982799b87480100084c94cc108cdc3998279129998250008a4000266e00c0ccdd5982b182a00098011829800804198279129998250008a4000266e00c0ccdd5982b182a000980118298008038a9982119baf00100915330423303200500615330423371266e08cdc080880f00a99b823370202802a03c266e24cdc119b8101101e0133370466e0404804c078c14400458c148008c12c004dd5005099191918191981998288008011bac30503035305102a3756609e606c60a0002609c048609a02a60980246eacc12c048dd5982500798248008b182500118218009baa0013045304400c30443043009375a6086605a608800e6eb4c108c0b0c10c03cdd69820981e18210029bad3040303f3041004375a607e607c60800186eb4c0f8c0fc008dd6981e981f0051981400b002981d000981d981d181c806181c000981c80099811000801181b181b8081bac30353034303400a153330333370ea66606666e1cdd6981a80624000290000a99981999b87375a606a01890010a40042a66606666e1cdd6981a80624008290020a99981999b87375a606a01890030a400c290042400c266e2400520d00f1323232323232323232323232323232323232323253330473370e90020010991919191929981f19b8700b00c1533304c3370ea66609866e1cdd69827012a4000290000a99982619b87375a609c04a90010a40042a66609866e1cdd69827012a4008290020a99982619b87375a609c04a90030a400c29004240002646464a66609e66e1d20040021325330423370e6609e44a66609400229000099b803033375660ac60a8002600460a60020106609e44a66609400229000099b803033375660ac60a8002600460a600200e2a6608466ebc00402454cc108cc0c801401854cc108cdc499b823370202203c02a66e08cdc080a00a80f099b893370466e0404407804ccdc119b8101201301e3051001163052002304b00137540142a66609866e1d4ccc130cdc39bad304e02548000520001533304c3370e6eb4c138095200214800854ccc130cdc39bad304e02548010520041533304c3370e6eb4c138095200614801852008480084c8c8c94ccc13ccdc3a4008004264a6608466e1ccc13c894ccc1280045200013370060666eacc158c150004c008c14c004020cc13c894ccc1280045200013370060666eacc158c150004c008c14c00401c54cc108cdd78008048a99821198190028030a9982119b893370466e04044078054cdc119b8101401501e13371266e08cdc080880f00999b823370202402603c60a20022c60a400460960026ea802854ccc130cdc3a99982619b87375a609c04a90000a40002a66609866e1cdd69827012a4004290010a99982619b87375a609c04a90020a40082a66609866e1cdd69827012a400c290030a40109002099191929982099b873304e2253330490011480004cdc018191bab30553053001300230520010073304e2253330490011480004cdc018191bab305530530013002305200100615330413371200266e08051200415330413303100400515330413370e66e0404007520001533041533304f33710900019b81013014133712605866e08cdc080880919b803370402805a66e08cdc080980a19b810030023370402466e08cdc080980a19b81003002133712605866e08cdc080980a19b803370402405a66e08cdc080880919b810030023370402866e08cdc080880919b810030021333222323232323232323232323253304f3303f3374a90001982b982f8059982b982f8051982b982f8049982b982f8041982b9ba8375a60be00e660ae6ea0dd6982f8031982b9ba8375a60be002660ae6ea0dd6982f982f0009982b9ba7375860be006660ae6ea0dd6982f8011982b9ba9375c60be60bc0040b201c2a6609ea6609e66e24cdc101da99982e99b88480000344cdc09bad305f001375a60be00a266e04dd6982f982f0009bad305f004533305d337109000006899b8200d375a60be00c266e08030dd6982f803099b88533305d337109000006899b8200d375a60be00c266e08030dd6982f80319b8203b33700a6660ba66e21200000d1337026eb4c17c004dd6982f802899b81375a60be60bc0026eb4c17c01120021533305d337109000006899b87375a60be0086eb4c17cc1780044cdc39bad305f005375a60be002608a60be01a60b800260b600260b400260b200260b000260ae00260ac00260aa00260a800260aa05c64646464018a6660a466e1d200000213232323232323232323232323232323232323232323232323232323232323232323232323232323232323232533307e3370e6e340052038132324994ccc1dc00452616307f00316375c00260fc00260f80066eb4004c1ec004c1e400ccc1588c8c8c8c80154ccc1eccdc3a400000426464646464646493299983c8008a4c2c61020200ca6660fe66e1d2000002132325333081013370e6e340052038132324994ccc1e80045261630820100316375c0026102020022a6660fe66e1d2002002132325333081013370e6e340052038132324994ccc1e80045261630820100316375c0026102020022c61040200460f60026ea8004c1f400454ccc1eccdc3a400400426464646464646464646493299983e0008a4c2c6108020066eb4004c20c04004c2040400cdd6800984000800983f0019bad001307d00116307e002307700137540026eb0004c1e0004c1d800cdd6800983a80098398019bad00130720013070003375a00260de00260da0066eb4004c1b0004c1a80194ccc1a0cdc3a40000042646464a6660d6a660b666e1c005200013370e002901c0991919299983719b89371a00290200991924ca6660ce0022930b18378018b1bae001306e001306c00416371a0026eb8004c1a800458c1ac008c190004dd500098330009832003299983119b87480000084c8c8c94ccc1954cc154cdc3800a4000266e1c005203813232325333068337126e340052040132324994ccc18400452616306900316375c00260d000260cc0082c6e34004dd700098320008b1832801182f0009baa0013060001305e006533305c3370e90000010991919299982fa9982799b87001480004cdc3800a40702646464a6660c466e24dc6800a40802646493299982d8008a4c2c60c60062c6eb8004c188004c18001058dc68009bae001305e00116305f0023058001375400260b400260b000ca6660ac66e1d2000002132323253330595330493370e0029000099b87001480e04c8c8c94ccc170cdc49b8d001481004c8c9265333055001149858c17400c58dd7000982e000982d0020b1b8d001375c00260b00022c60b200460a40026ea8004c15000458c154008c138004dd500419b81013014337020220246eb4c140c0b8c1440a8dd69827981b98280149bad304e3037304f0281533304c3370ea66609866e1cdd69827012a4000290000a99982619b87375a609c04a90010a40042a66609866e1cdd69827012a4008290020a99982619b87375a609c04a90030a400c290042400c2646464a66609e66e1d20040021325330423370e6609e44a66609400229000099b803033375660ac60a8002600460a60020106609e44a66609400229000099b803033375660ac60a8002600460a600200e2a6608466ebc00402454cc108cc0c801401854cc108cdc499b823370202203c02a66e08cdc080a00a80f099b893370466e0404407804ccdc119b8101201301e3051001163052002304b0013754014264646460646606660a20020046eb0c140c0d4c1440a8dd59827981b18280009827012182680a98260091bab304b0123756609401e60920022c609400460860026ea8004c114c110030c110c10c024dd69821981698220039bad3042302c304300f375a60826078608400a6eb4c100c0fcc104010dd6981f981f18200061bad303e303f002375a607a607c0146605002c00a60740026076607460720186070002607200266044002004606c606e0206eb0c0d4c0d0c0d0028dd6981a1817981a8009980f8069819981900298190008b181980118160009baa302f302e005302f0013322533302d3371000490000b0999816111299981819b87002480004c0c80044cc00ccdc08012400460620020040026eb4c0b4c0b0010004dd6181600098161815800981580118150010a4c2c466e0520000014830268308c084894ccc07000440804cc078c00cc098004c008c0940048c088c020004cc0788894ccc06800440084cc00ccdc000124004604600290001119baf374e60460046e9cc08c0048cc0049288a502330020030012223301d2253330180011225001153330203375e603c60440020082600a60440022600460420020024644460040066eb4c07c0048c06cc0080048c068c0080048c064c0080048c060c0080048c05cc0080048c058c0480048c04c894ccc0380045854ccc058cdc399805191bab30193018301a0013018001003480084c0600044c008c05c00488c8c8c8c8c8cdd2a4000660266ea0cdc099806800980d8029bad301b00233013375066e04cc034004c06c010dd6980d980d001198099ba8337020106601a0026036006660266ea14ccc0654cc024c8c94cc034cdc79bae301d301c002375c603a6038002266e3cdd7180e8011bae301d001301d013301c301b0051323253300d3371e6eb8c074c070008dd7180e980e000899b8f375c603a0046eb8c074004c07404cc070c06c0105200013300d001012015375660346032603600a60286030002602e002602c002602a602e004907f7fffffffffffffff809198088008010a512233301000200100314a044646660080066eb8c044004dd71808980800098088009111999802001240004666600a00490003ad3756002006460046ea40048888cc030894ccc01c004401454ccc03ccdd7980698088008030980218099808800898011808000800aab9f3374a9000198009ba9002330013752004006ae812201004bd70118029802800aab9d2323002233002002001230022330020020015734ae895d0918011baa0015573d",
        )
    }

    fn reference_utxo_from_cbor(tx_hash: &str, index: u64, txout_cbor: &str) -> TransactionUnspentOutput {
        TransactionUnspentOutput::new(
            OutputRef::new(TransactionHash::from_hex(tx_hash).unwrap(), index).into(),
            TransactionOutput::from_cbor_bytes(&hex::decode(txout_cbor).unwrap()).unwrap(),
        )
    }

    impl Has<OutputRef> for ExactLedgerContext {
        fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
            self.oref
        }
    }

    impl Has<ConsumedInputs> for ExactLedgerContext {
        fn select<U: IsEqual<ConsumedInputs>>(&self) -> ConsumedInputs {
            self.consumed_inputs
        }
    }

    impl Has<ConsumedIdentifiers<Token>> for ExactLedgerContext {
        fn select<U: IsEqual<ConsumedIdentifiers<Token>>>(&self) -> ConsumedIdentifiers<Token> {
            self.consumed_identifiers
        }
    }

    impl Has<OperatorCred> for ExactLedgerContext {
        fn select<U: IsEqual<OperatorCred>>(&self) -> OperatorCred {
            self.cred
        }
    }

    impl Has<LimitOrderValidation> for ExactLedgerContext {
        fn select<U: IsEqual<LimitOrderValidation>>(&self) -> LimitOrderValidation {
            self.limit_validation
        }
    }

    impl Has<PoolValidation> for ExactLedgerContext {
        fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
            self.pool_validation
        }
    }

    impl Has<NetworkId> for ExactLedgerContext {
        fn select<U: IsEqual<NetworkId>>(&self) -> NetworkId {
            self.network_id
        }
    }

    impl Has<OperatorRewardAddress> for ExactLedgerContext {
        fn select<U: IsEqual<OperatorRewardAddress>>(&self) -> OperatorRewardAddress {
            self.operator_reward.clone()
        }
    }

    impl Has<Collateral> for ExactLedgerContext {
        fn select<U: IsEqual<Collateral>>(&self) -> Collateral {
            self.collateral.clone()
        }
    }

    impl Has<DeployedScriptInfo<{ LimitOrderV1 as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ LimitOrderV1 as u8 }> {
            self.scripts.limit_order
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolV1 as u8 }> {
            self.scripts.const_fn_pool_v1
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolV2 as u8 }> {
            self.scripts.const_fn_pool_v2
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }> {
            self.scripts.const_fn_pool_fee_switch
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }> {
            self.scripts.const_fn_pool_fee_switch_v2
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
            self.scripts.const_fn_pool_fee_switch_bidir_fee
        }
    }

    impl Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }> {
            self.scripts.balance_fn_pool_v1
        }
    }

    impl Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }> {
            self.scripts.balance_fn_pool_v2
        }
    }

    impl Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ StableFnPoolT2T as u8 }> {
            self.scripts.stable_fn_pool_t2t
        }
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }> {
            self.scripts.royalty_pool_v1
        }
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }> {
            self.scripts.royalty_pool_v1_ledger_fixed
        }
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }> {
            self.scripts.royalty_pool_v2
        }
    }

    impl Has<DeployedValidator<{ LimitOrderV1 as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedValidator<{ LimitOrderV1 as u8 }>>>(
            &self,
        ) -> DeployedValidator<{ LimitOrderV1 as u8 }> {
            DeployedValidator {
                reference_utxo: actual_limit_order_reference_utxo(),
                hash: self.deployment.limit_order.hash,
                cost: self.deployment.limit_order.cost,
                marginal_cost: self
                    .deployment
                    .limit_order
                    .marginal_cost
                    .unwrap_or(ExUnits::empty()),
            }
        }
    }

    impl Has<DeployedValidator<{ LimitOrderWitnessV1 as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedValidator<{ LimitOrderWitnessV1 as u8 }>>>(
            &self,
        ) -> DeployedValidator<{ LimitOrderWitnessV1 as u8 }> {
            DeployedValidator {
                reference_utxo: self.mock_reference_utxo.clone(),
                hash: self.deployment.limit_order_witness.hash,
                cost: self.deployment.limit_order_witness.cost,
                marginal_cost: self
                    .deployment
                    .limit_order_witness
                    .marginal_cost
                    .unwrap_or(ExUnits::empty()),
            }
        }
    }

    macro_rules! exact_ctx_validator {
        ($variant:ident, $field:ident) => {
            impl Has<DeployedValidator<{ $variant as u8 }>> for ExactLedgerContext {
                fn select<U: IsEqual<DeployedValidator<{ $variant as u8 }>>>(
                    &self,
                ) -> DeployedValidator<{ $variant as u8 }> {
                    DeployedValidator {
                        reference_utxo: self.mock_reference_utxo.clone(),
                        hash: self.deployment.$field.hash,
                        cost: self.deployment.$field.cost,
                        marginal_cost: self
                            .deployment
                            .$field
                            .marginal_cost
                            .unwrap_or(ExUnits::empty()),
                    }
                }
            }
        };
    }

    exact_ctx_validator!(ConstFnPoolV1, const_fn_pool_v1);
    exact_ctx_validator!(ConstFnPoolV2, const_fn_pool_v2);
    exact_ctx_validator!(ConstFnPoolFeeSwitch, const_fn_pool_fee_switch);
    exact_ctx_validator!(ConstFnPoolFeeSwitchBiDirFee, const_fn_pool_fee_switch_bidir_fee);
    exact_ctx_validator!(RoyaltyPoolV1, royalty_pool);
    exact_ctx_validator!(RoyaltyPoolV1LedgerFixed, royalty_pool_ledger_fixed);
    exact_ctx_validator!(RoyaltyPoolV2, royalty_pool_v2);
    exact_ctx_validator!(BalanceFnPoolV1, balance_fn_pool_v1);
    exact_ctx_validator!(BalanceFnPoolV2, balance_fn_pool_v2);
    exact_ctx_validator!(StableFnPoolT2T, stable_fn_pool_t2t);

    impl Has<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>> for ExactLedgerContext {
        fn select<U: IsEqual<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>>>(
            &self,
        ) -> DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }> {
            DeployedValidator {
                reference_utxo: actual_const_fn_pool_fee_switch_v2_reference_utxo(),
                hash: self.deployment.const_fn_pool_fee_switch_v2.hash,
                cost: self.deployment.const_fn_pool_fee_switch_v2.cost,
                marginal_cost: self
                    .deployment
                    .const_fn_pool_fee_switch_v2
                    .marginal_cost
                    .unwrap_or(ExUnits::empty()),
            }
        }
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
