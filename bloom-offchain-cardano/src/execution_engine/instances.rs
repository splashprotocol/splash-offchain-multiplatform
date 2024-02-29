use std::hash::Hash;

use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::{ConstrPlutusData, PlutusData, RedeemerTag};
use cml_chain::transaction::TransactionInput;
use cml_chain::utils::BigInt;
use cml_crypto::TransactionHash;
use log::trace;
use spectrum_offchain_cardano::data::pair::order_canonical;
use void::Void;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::liquidity_book::fragment::StateTrans;
use bloom_offchain::execution_engine::liquidity_book::recipe::{LinkedFill, LinkedSwap};
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use spectrum_cardano_lib::output::{FinalizedTxOut, IndexedTxOut};
use spectrum_cardano_lib::plutus_data::RequiresRedeemer;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::constants::POOL_EXECUTION_UNITS;
use spectrum_offchain_cardano::data::pool::{CFMMPoolAction, CFMMPoolRefScriptOutput, ClassicCFMMPool};
use spectrum_offchain_cardano::data::PoolVer;

use crate::execution_engine::execution_state::ExecutionState;
use crate::orders::spot::{
    unsafe_update_n2t_variables, SpotOrder, SpotOrderRefScriptOutput, SPOT_ORDER_N2T_EX_UNITS,
};
use crate::orders::AnyOrder;
use crate::pools::AnyPool;

/// Magnet for local instances.
#[repr(transparent)]
pub struct Magnet<T>(pub T);

impl<Ctx> BatchExec<ExecutionState, (TypedTransactionInput, Option<IndexedTxOut>), Ctx, Void>
    for Magnet<LinkedFill<AnyOrder, FinalizedTxOut>>
where
    Ctx: Has<SpotOrderRefScriptOutput>,
{
    fn try_exec(
        self,
        state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, (TypedTransactionInput, Option<IndexedTxOut>), Ctx), Void> {
        match self.0 {
            LinkedFill {
                target_fr: Bundled(AnyOrder::Spot(o), src),
                next_fr: transition,
                removed_input,
                added_output,
                budget_used,
                fee_used,
            } => Magnet(LinkedFill {
                target_fr: Bundled(o, src),
                next_fr: transition.map(|AnyOrder::Spot(o2)| o2),
                removed_input,
                added_output,
                budget_used,
                fee_used,
            })
            .try_exec(state, context),
        }
    }
}

impl<Ctx> BatchExec<ExecutionState, (TypedTransactionInput, Option<IndexedTxOut>), Ctx, Void>
    for Magnet<LinkedFill<SpotOrder, FinalizedTxOut>>
where
    Ctx: Has<SpotOrderRefScriptOutput>,
{
    fn try_exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, (TypedTransactionInput, Option<IndexedTxOut>), Ctx), Void> {
        let Magnet(LinkedFill {
            target_fr: Bundled(ord, FinalizedTxOut(consumed_out, in_ref)),
            next_fr: transition,
            removed_input,
            added_output,
            budget_used,
            fee_used,
        }) = self;
        let mut candidate = consumed_out.clone();
        // Subtract budget + fee used to facilitate execution.
        candidate.sub_asset(ord.fee_asset, budget_used + fee_used);
        // Subtract tradable input used in exchange.
        candidate.sub_asset(ord.input_asset, removed_input);
        // Add output resulted from exchange.
        candidate.add_asset(ord.output_asset, added_output);
        let residual_order = {
            let mut candidate = candidate.clone();
            match transition {
                StateTrans::Active(next) => {
                    if let Some(data) = candidate.data_mut() {
                        unsafe_update_n2t_variables(data, next.input_amount, next.fee);
                    }
                    Some(candidate)
                }
                StateTrans::EOL => {
                    candidate.null_datum();
                    candidate.update_payment_cred(ord.redeemer_cred());
                    None
                }
            }
        };
        let successor_ix = state.tx_builder.num_outputs();
        let order_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(candidate.script_hash().unwrap()),
            spot_exec_redeemer(successor_ix as u16),
        );
        let spot_order_ref_script = context.get_labeled::<SpotOrderRefScriptOutput>().0;
        state.tx_builder.add_reference_input(spot_order_ref_script);
        let order_in = SingleInputBuilder::new(in_ref.into(), consumed_out)
            .plutus_script_inline_datum(order_script, Vec::new())
            .unwrap();
        state
            .tx_builder
            .add_output(SingleOutputBuilderResult::new(candidate))
            .unwrap();
        let indexed_tx_in = TypedTransactionInput::SpotOrder(order_in.input.clone());
        state.tx_builder.add_input(order_in).unwrap();
        state.add_ex_budget(ord.fee_asset, budget_used);
        Ok((
            state,
            (
                indexed_tx_in,
                residual_order.map(|o| IndexedTxOut(successor_ix, o)),
            ),
            context,
        ))
    }
}

fn spot_exec_redeemer(successor_ix: u16) -> PlutusData {
    PlutusData::ConstrPlutusData(ConstrPlutusData::new(
        0,
        vec![PlutusData::Integer(BigInt::from(successor_ix))],
    ))
}

/// Batch execution routing for [AnyPool].
impl<Ctx> BatchExec<ExecutionState, (TypedTransactionInput, IndexedTxOut), Ctx, Void>
    for Magnet<LinkedSwap<AnyPool, FinalizedTxOut>>
where
    Ctx: Has<CFMMPoolRefScriptOutput<1>> + Has<CFMMPoolRefScriptOutput<2>>,
{
    fn try_exec(
        self,
        state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, (TypedTransactionInput, IndexedTxOut), Ctx), Void> {
        match self.0 {
            LinkedSwap {
                target: Bundled(AnyPool::CFMM(p), src),
                transition: AnyPool::CFMM(p2),
                side,
                input,
                output,
            } => Magnet(LinkedSwap {
                target: Bundled(p, src),
                transition: p2,
                side,
                input,
                output,
            })
            .try_exec(state, context),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TypedTransactionInput {
    CFMMPool(TransactionInput),
    SpotOrder(TransactionInput),
}

impl TypedTransactionInput {
    pub fn get_inner(&self) -> TransactionInput {
        match self {
            TypedTransactionInput::CFMMPool(t) | TypedTransactionInput::SpotOrder(t) => t.clone(),
        }
    }
}

/// Batch execution logic for [ClassicCFMMPool].
impl<Ctx> BatchExec<ExecutionState, (TypedTransactionInput, IndexedTxOut), Ctx, Void>
    for Magnet<LinkedSwap<ClassicCFMMPool, FinalizedTxOut>>
where
    Ctx: Has<CFMMPoolRefScriptOutput<1>> + Has<CFMMPoolRefScriptOutput<2>>,
{
    fn try_exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, (TypedTransactionInput, IndexedTxOut), Ctx), Void> {
        let Magnet(LinkedSwap {
            target: Bundled(pool, FinalizedTxOut(consumed_out, in_ref)),
            side,
            input,
            output,
            ..
        }) = self;
        let mut produced_out = consumed_out.clone();
        let x = pool.asset_x.untag();
        let y = pool.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        let (removed_asset, added_asset) = if base == x {
            match side {
                SideM::Bid => (x, y),
                SideM::Ask => (y, x),
            }
        } else {
            match side {
                SideM::Bid => (y, x),
                SideM::Ask => (x, y),
            }
        };
        produced_out.sub_asset(removed_asset, output);
        produced_out.add_asset(added_asset, input);
        let successor = produced_out.clone();
        let successor_ix = state.tx_builder.output_sizes().len();
        let pool_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(produced_out.script_hash().unwrap()),
            ClassicCFMMPool::redeemer(CFMMPoolAction::Swap),
        );
        let pool_in = SingleInputBuilder::new(in_ref.into(), consumed_out)
            .plutus_script_inline_datum(pool_script, Vec::new())
            .unwrap();
        state
            .tx_builder
            .add_output(SingleOutputBuilderResult::new(produced_out))
            .unwrap();
        let pool_ref_script = match pool.ver {
            PoolVer::V1 => context.get_labeled::<CFMMPoolRefScriptOutput<1>>().0,
            _ => context.get_labeled::<CFMMPoolRefScriptOutput<2>>().0,
        };
        state.tx_builder.add_reference_input(pool_ref_script);
        let indexed_tx_in = TypedTransactionInput::CFMMPool(pool_in.input.clone());
        let indexed_tx_out = IndexedTxOut(successor_ix, successor);
        state.tx_builder.add_input(pool_in).unwrap();
        Ok((state, (indexed_tx_in, indexed_tx_out), context))
    }
}
