use cml_chain::transaction::TransactionOutput;
use void::Void;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::execution_effect::ExecutionEff;
use bloom_offchain::execution_engine::liquidity_book::fragment::StateTrans;
use bloom_offchain::execution_engine::liquidity_book::recipe::{LinkedFill, LinkedSwap};
use spectrum_cardano_lib::NetworkId;
use spectrum_cardano_lib::output::FinalizedTxOut;

use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::data::balance_pool::{BalancePool, BalancePoolRedeemer};
use spectrum_offchain_cardano::data::cfmm_pool::{CFMMPoolRedeemer, ConstFnPool};
use spectrum_offchain_cardano::data::pool::{AnyPool, AssetDeltas, CFMMPoolAction};
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolV1, ConstFnPoolV1, ConstFnPoolV2, LimitOrderV1,
};
use spectrum_offchain_cardano::deployment::{DeployedValidator, RequiresValidator};

use crate::execution_engine::execution_state::{
    delayed_redeemer, ready_redeemer, ExecutionState, ScriptInputBlueprint,
};
use crate::orders::spot::{unsafe_update_n2t_variables, LimitOrder};
use crate::orders::{spot, AnyOrder};

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
    Ctx: Has<NetworkId> + Has<DeployedValidator<{ LimitOrderV1 as u8 }>>,
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

impl<Ctx> BatchExec<ExecutionState, OrderResult<LimitOrder>, Ctx, Void>
    for Magnet<LinkedFill<LimitOrder, FinalizedTxOut>>
where
    Ctx: Has<NetworkId> + Has<DeployedValidator<{ LimitOrderV1 as u8 }>>,
{
    fn try_exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, OrderResult<LimitOrder>, Ctx), Void> {
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
            script: context.select::<DeployedValidator<{ LimitOrderV1 as u8 }>>().erased(),
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
                    candidate.update_address(ord.redeemer_address.to_address(context.select::<NetworkId>()));
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
    Ctx: Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>,
{
    fn try_exec(
        self,
        state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, PoolResult<AnyPool>, Ctx), Void> {
        match self.0 {
            LinkedSwap {
                target: Bundled(AnyPool::PureCFMM(p), src),
                transition: AnyPool::PureCFMM(p2),
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
            .map(|(st, res, ctx)| (st, res.map(AnyPool::PureCFMM), ctx)),
            LinkedSwap {
                target: Bundled(AnyPool::BalancedCFMM(p), src),
                transition: AnyPool::BalancedCFMM(p2),
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
            .map(|(st, res, ctx)| (st, res.map(AnyPool::BalancedCFMM), ctx)),
            _ => unreachable!(),
        }
    }
}

/// Batch execution logic for [ConstFnPool].
impl<Ctx> BatchExec<ExecutionState, PoolResult<ConstFnPool>, Ctx, Void>
    for Magnet<LinkedSwap<ConstFnPool, FinalizedTxOut>>
where
    Ctx: Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>> + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>,
{
    fn try_exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, PoolResult<ConstFnPool>, Ctx), Void> {
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

        let pool_validator = pool.get_validator(&context);
        let input = ScriptInputBlueprint {
            reference: in_ref,
            utxo: consumed_out,
            script: pool_validator,
            redeemer: delayed_redeemer(move |ordering| {
                CFMMPoolRedeemer {
                    pool_input_index: ordering.index_of(&in_ref) as u64,
                    action: CFMMPoolAction::Swap,
                }
                .to_plutus_data()
            }),
        };
        let result = Bundled(transition, produced_out.clone());

        state.tx_blueprint.add_io(input, produced_out);
        Ok((state, result, context))
    }
}

impl<Ctx> BatchExec<ExecutionState, PoolResult<BalancePool>, Ctx, Void>
    for Magnet<LinkedSwap<BalancePool, FinalizedTxOut>>
where
    Ctx: Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>,
{
    fn try_exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, PoolResult<BalancePool>, Ctx), Void> {
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

        let pool_validator = pool.get_validator(&context);
        let input = ScriptInputBlueprint {
            reference: in_ref,
            utxo: consumed_out,
            script: pool_validator,
            redeemer: delayed_redeemer(move |ordering| {
                BalancePoolRedeemer {
                    pool_input_index: ordering.index_of(&in_ref) as u64,
                    action: CFMMPoolAction::Swap,
                    new_pool_state: transition,
                    prev_pool_state: pool,
                }
                .to_plutus_data()
            }),
        };
        let result = Bundled(transition, produced_out.clone());

        state.tx_blueprint.add_io(input, produced_out);
        Ok((state, result, context))
    }
}
