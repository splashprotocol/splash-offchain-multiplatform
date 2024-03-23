use cml_chain::transaction::TransactionOutput;
use void::Void;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::execution_effect::ExecutionEff;
use bloom_offchain::execution_engine::liquidity_book::fragment::StateTrans;
use bloom_offchain::execution_engine::liquidity_book::recipe::{LinkedFill, LinkedSwap};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::plutus_data::RequiresRedeemer;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::data::pool::{AssetDeltas, CFMMPool, CFMMPoolAction};
use spectrum_offchain_cardano::data::PoolVer;
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{ConstFnPoolV1, ConstFnPoolV2, LimitOrder};

use crate::execution_engine::execution_state::{
    delayed_redeemer, ready_redeemer, ExecutionState, ScriptInputBlueprint,
};
use crate::orders::spot::{unsafe_update_n2t_variables, SpotOrder};
use crate::orders::{spot, AnyOrder};
use crate::pools::AnyPool;

/// Magnet for local instances.
#[repr(transparent)]
pub struct Magnet<T>(pub T);

/// Result of order execution.
pub type OrderResult<Order> = ExecutionEff<Bundled<Order, TransactionOutput>, Bundled<Order, FinalizedTxOut>>;
/// Result of operation applied to a pool.
pub type PoolResult<Pool> = Bundled<Pool, TransactionOutput>;

impl<Ctx> BatchExec<ExecutionState, OrderResult<AnyOrder>, Ctx, Void>
    for Magnet<LinkedFill<AnyOrder, FinalizedTxOut>>
where
    Ctx: Has<DeployedValidator<{ LimitOrder as u8 }>>,
{
    fn try_exec(
        self,
        state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, OrderResult<AnyOrder>, Ctx), Void> {
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
            .try_exec(state, context)
            .map(|(st, res, ctx)| {
                (
                    st,
                    res.bimap(|u| u.map(AnyOrder::Spot), |e| e.map(AnyOrder::Spot)),
                    ctx,
                )
            }),
        }
    }
}

impl<Ctx> BatchExec<ExecutionState, OrderResult<SpotOrder>, Ctx, Void>
    for Magnet<LinkedFill<SpotOrder, FinalizedTxOut>>
where
    Ctx: Has<DeployedValidator<{ LimitOrder as u8 }>>,
{
    fn try_exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, OrderResult<SpotOrder>, Ctx), Void> {
        let Magnet(LinkedFill {
            target_fr: Bundled(ord, FinalizedTxOut(consumed_out, in_ref)),
            next_fr: transition,
            removed_input,
            added_output,
            budget_used,
            fee_used,
        }) = self;
        let input = ScriptInputBlueprint {
            reference: in_ref,
            utxo: consumed_out.clone(),
            script: context.get().erased(),
            redeemer: ready_redeemer(spot::EXEC_REDEEMER),
        };
        let mut candidate = consumed_out.clone();
        // Subtract budget + fee used to facilitate execution.
        candidate.sub_asset(ord.fee_asset, budget_used + fee_used);
        // Subtract tradable input used in exchange.
        candidate.sub_asset(ord.input_asset, removed_input);
        // Add output resulted from exchange.
        candidate.add_asset(ord.output_asset, added_output);
        let (residual_order, effect) = {
            match transition {
                StateTrans::Active(next) => {
                    if let Some(data) = candidate.data_mut() {
                        unsafe_update_n2t_variables(data, next.input_amount, next.fee);
                    }
                    (candidate.clone(), ExecutionEff::Updated(Bundled(next, candidate)))
                }
                StateTrans::EOL => {
                    candidate.null_datum();
                    candidate.update_payment_cred(ord.redeemer_cred());
                    (
                        candidate,
                        ExecutionEff::Eliminated(Bundled(ord, FinalizedTxOut(consumed_out, in_ref))),
                    )
                }
            }
        };
        state.tx_blueprint.add_io(input, residual_order);
        state.add_ex_budget(ord.fee_asset, budget_used);
        Ok((state, effect, context))
    }
}

/// Batch execution routing for [AnyPool].
impl<Ctx> BatchExec<ExecutionState, PoolResult<AnyPool>, Ctx, Void>
    for Magnet<LinkedSwap<AnyPool, FinalizedTxOut>>
where
    Ctx: Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>> + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>,
{
    fn try_exec(
        self,
        state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, PoolResult<AnyPool>, Ctx), Void> {
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
            .try_exec(state, context)
            .map(|(st, res, ctx)| (st, res.map(AnyPool::CFMM), ctx)),
        }
    }
}

/// Batch execution logic for [CFMMPool].
impl<Ctx> BatchExec<ExecutionState, PoolResult<CFMMPool>, Ctx, Void>
    for Magnet<LinkedSwap<CFMMPool, FinalizedTxOut>>
where
    Ctx: Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>> + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>,
{
    fn try_exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, PoolResult<CFMMPool>, Ctx), Void> {
        let Magnet(LinkedSwap {
            target: Bundled(pool, FinalizedTxOut(consumed_out, in_ref)),
            transition,
            side,
            input,
            output,
        }) = self;
        let mut produced_out = consumed_out.clone();
        let AssetDeltas {
            asset_to_deduct_from,
            asset_to_add_to,
        } = pool.get_asset_deltas(side);
        produced_out.sub_asset(asset_to_deduct_from, output);
        produced_out.add_asset(asset_to_add_to, input);

        let pool_validator = match pool.ver {
            PoolVer::V1 => context
                .get_labeled::<DeployedValidator<{ ConstFnPoolV1 as u8 }>>()
                .erased(),
            _ => context
                .get_labeled::<DeployedValidator<{ ConstFnPoolV2 as u8 }>>()
                .erased(),
        };
        let input = ScriptInputBlueprint {
            reference: in_ref,
            utxo: consumed_out,
            script: pool_validator,
            redeemer: delayed_redeemer(move |ordering| {
                CFMMPool::redeemer(CFMMPoolAction::Swap, ordering.index_of(&in_ref))
            }),
        };
        let result = Bundled(transition, produced_out.clone());

        state.tx_blueprint.add_io(input, produced_out);
        Ok((state, result, context))
    }
}
