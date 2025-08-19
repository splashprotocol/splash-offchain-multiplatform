use cml_chain::plutus::PlutusData;
use cml_chain::transaction::TransactionOutput;
use cml_crypto::Ed25519KeyHash;
use log::trace;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::execution_effect::ExecutionEff;
use bloom_offchain::execution_engine::liquidity_book::core::{Make, Next, Take, Trans};
use bloom_offchain::execution_engine::liquidity_book::market_taker::MarketTaker;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, NetworkId};
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::data::balance_pool::{BalancePool, BalancePoolRedeemer};
use spectrum_offchain_cardano::data::cfmm_pool::{CFMMPoolRedeemer, ConstFnPool};
use spectrum_offchain_cardano::data::pool::{AnyPool, CFMMPoolAction, PoolAssetMapping};
use spectrum_offchain_cardano::data::quadratic_pool::QuadraticPoolVer::V1T2T;
use spectrum_offchain_cardano::data::quadratic_pool::{QuadraticPool, QuadraticPoolRedeemer};
use spectrum_offchain_cardano::data::stable_pool_t2t::{StablePoolRedeemer, StablePoolT2T};
use spectrum_offchain_cardano::data::{balance_pool, cfmm_pool, quadratic_pool, stable_pool_t2t};
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolV1, BalanceFnPoolV2, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee,
    ConstFnPoolFeeSwitchV2, ConstFnPoolV1, ConstFnPoolV2, DegenQuadraticPoolV1, DegenQuadraticPoolV1T2T,
    GridOrderNative, InstantOrderV1, InstantOrderWitnessV1, LimitOrderV1, LimitOrderWitnessV1, RoyaltyPoolV1,
    RoyaltyPoolV1LedgerFixed, RoyaltyPoolV2, StableFnPoolT2T,
};
use spectrum_offchain_cardano::deployment::{DeployedValidator, DeployedValidatorErased, RequiresValidator};
use spectrum_offchain_cardano::script::{
    delayed_cost, delayed_redeemer, ready_cost, ready_redeemer, ScriptWitness,
};

use crate::execution_engine::execution_state::{ExecutionState, ScriptInputBlueprint};
use crate::orders::adhoc::{AdhocFeeStructure, AdhocOrder};
use crate::orders::grid::GridOrder;
use crate::orders::limit::LimitOrder;
use crate::orders::{grid, instant, limit, AnyOrder};

/// Magnet for local instances.
#[repr(transparent)]
pub struct Magnet<T>(pub T);

pub type EffectPreview<T> = ExecutionEff<Bundled<T, TransactionOutput>, Bundled<T, FinalizedTxOut>>;
pub type FinalizedEffect<T> = ExecutionEff<Bundled<T, FinalizedTxOut>, Bundled<T, FinalizedTxOut>>;

impl<Ctx> BatchExec<ExecutionState, EffectPreview<AnyOrder>, Ctx> for Magnet<Take<AnyOrder, FinalizedTxOut>>
where
    Ctx: Has<NetworkId>
        + Has<OperatorCred>
        + Has<DeployedValidator<{ GridOrderNative as u8 }>>
        + Has<DeployedValidator<{ LimitOrderV1 as u8 }>>
        + Has<DeployedValidator<{ LimitOrderWitnessV1 as u8 }>>,
{
    fn exec(self, state: ExecutionState, context: Ctx) -> (ExecutionState, EffectPreview<AnyOrder>, Ctx) {
        match self {
            Magnet(Trans {
                target: Bundled(AnyOrder::Limit(o), src),
                result,
            }) => {
                let (st, res, ctx) = Magnet(Trans {
                    target: Bundled(o, src),
                    result: result.map_succ(|ord| match ord {
                        AnyOrder::Limit(o2) => o2,
                        _ => unreachable!(),
                    }),
                })
                .exec(state, context);
                (
                    st,
                    res.bimap(|u| u.map(AnyOrder::Limit), |e| e.map(AnyOrder::Limit)),
                    ctx,
                )
            }
            Magnet(Trans {
                target: Bundled(AnyOrder::Grid(o), src),
                result,
            }) => {
                let (st, res, ctx) = Magnet(Trans {
                    target: Bundled(o, src),
                    result: result.map_succ(|ord| match ord {
                        AnyOrder::Grid(o2) => o2,
                        _ => unreachable!(),
                    }),
                })
                .exec(state, context);
                (
                    st,
                    res.bimap(|u| u.map(AnyOrder::Grid), |e| e.map(AnyOrder::Grid)),
                    ctx,
                )
            }
        }
    }
}

impl<Ctx> BatchExec<ExecutionState, EffectPreview<LimitOrder>, Ctx>
    for Magnet<Take<LimitOrder, FinalizedTxOut>>
where
    Ctx: Has<NetworkId>
        + Has<OperatorCred>
        + Has<DeployedValidator<{ LimitOrderV1 as u8 }>>
        + Has<DeployedValidator<{ LimitOrderWitnessV1 as u8 }>>,
{
    fn exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> (ExecutionState, EffectPreview<LimitOrder>, Ctx) {
        let Magnet(trans) = self;
        trace!("Running transition: {}", trans);
        let removed_input = trans.removed_input();
        let added_output = trans.added_output();
        let consumed_budget = trans.consumed_budget();
        let consumed_fee = trans.consumed_fee();
        trace!(
            "LimitOrder::exec(removed_input={}, added_output={}, consumed_budget={}, consumed_fee={})",
            removed_input,
            added_output,
            consumed_budget,
            consumed_fee
        );
        let Trans {
            target: Bundled(ord, FinalizedTxOut(consumed_out, in_ref)),
            result,
        } = trans;
        let DeployedValidatorErased {
            reference_utxo,
            hash,
            ex_budget,
            ..
        } = context
            .select::<DeployedValidator<{ LimitOrderV1 as u8 }>>()
            .erased();
        let input = ScriptInputBlueprint {
            reference: in_ref,
            utxo: consumed_out.clone(),
            script: ScriptWitness {
                hash,
                cost: ready_cost(ex_budget),
            },
            redeemer: ready_redeemer(limit::EXEC_REDEEMER),
            required_signers: if ord.requires_executor_sig {
                vec![Ed25519KeyHash::from(context.select::<OperatorCred>())].into()
            } else {
                vec![].into()
            },
        };
        let mut candidate = consumed_out.clone();
        // Subtract budget + fee used to facilitate execution.
        candidate.sub_asset(ord.fee_asset, consumed_budget + consumed_fee);
        // Subtract tradable input used in exchange.
        candidate.sub_asset(ord.input_asset, removed_input);
        // Add output resulted from exchange.
        candidate.add_asset(ord.output_asset, added_output);
        let consumed_bundle = Bundled(ord, FinalizedTxOut(consumed_out, in_ref));
        let (residual_order, effect) = match result {
            Next::Succ(next) => {
                if let Some(data) = candidate.data_mut() {
                    limit::unsafe_update_datum(data, next.input_amount, next.fee);
                }
                (
                    candidate.clone(),
                    ExecutionEff::Updated(consumed_bundle, Bundled(next, candidate)),
                )
            }
            Next::Term(_) => {
                candidate.null_datum();
                candidate.update_address(ord.redeemer_address.to_address(context.select::<NetworkId>()));
                (candidate, ExecutionEff::Eliminated(consumed_bundle))
            }
        };
        let witness = context.select::<DeployedValidator<{ LimitOrderWitnessV1 as u8 }>>();
        state.add_tx_fee(consumed_budget);
        state.add_operator_interest(consumed_fee);
        state
            .tx_blueprint
            .add_witness(witness.erased(), PlutusData::new_list(vec![]));
        state.tx_blueprint.add_io(input, residual_order);
        state.tx_blueprint.add_ref_input(reference_utxo);
        (state, effect, context)
    }
}

impl<Ctx> BatchExec<ExecutionState, EffectPreview<AdhocOrder>, Ctx>
    for Magnet<Take<AdhocOrder, FinalizedTxOut>>
where
    Ctx: Has<NetworkId>
        + Has<OperatorCred>
        + Has<DeployedValidator<{ InstantOrderV1 as u8 }>>
        + Has<DeployedValidator<{ InstantOrderWitnessV1 as u8 }>>
        + Has<AdhocFeeStructure>,
{
    fn exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> (ExecutionState, EffectPreview<AdhocOrder>, Ctx) {
        let Magnet(trans) = self;
        trace!("Running transition: {}", trans);
        let removed_input = trans.removed_input();
        let added_output = trans.added_output();
        let consumed_budget = trans.consumed_budget();
        let consumed_fee = trans.consumed_fee();
        trace!(
            "AdhocOrder::exec(removed_input={}, added_output={}, consumed_budget={}, consumed_fee={})",
            removed_input,
            added_output,
            consumed_budget,
            consumed_fee
        );
        let Trans {
            target: Bundled(AdhocOrder(ord, adhoc_fee_input), FinalizedTxOut(consumed_utxo, in_ref)),
            result,
        } = trans;
        let DeployedValidatorErased {
            reference_utxo,
            hash,
            ex_budget,
            ..
        } = context
            .select::<DeployedValidator<{ InstantOrderV1 as u8 }>>()
            .erased();
        let input = ScriptInputBlueprint {
            reference: in_ref,
            utxo: consumed_utxo.clone(),
            script: ScriptWitness {
                hash,
                cost: ready_cost(ex_budget),
            },
            redeemer: ready_redeemer(instant::EXEC_REDEEMER),
            required_signers: vec![Ed25519KeyHash::from(context.select::<OperatorCred>())].into(),
        };
        let full_adhoc_fee = match (ord.input_asset, ord.output_asset) {
            (AssetClass::Native, _) => adhoc_fee_input,
            (_, AssetClass::Native) => context.select::<AdhocFeeStructure>().fee(added_output),
            _ => 0,
        };
        let proportional_fee = (full_adhoc_fee as u128 * removed_input as u128 / ord.input() as u128) as u64;
        trace!(
            "consumed_budget: {}, consumed_fee: {}, full_adhoc_fee: {}, proportional_fee: {}",
            consumed_budget,
            consumed_fee,
            full_adhoc_fee,
            proportional_fee
        );
        let mut candidate = consumed_utxo.clone();
        // Subtract tradable input used in exchange.
        candidate.sub_asset(ord.input_asset, removed_input);
        // Add output resulted from exchange.
        candidate.add_asset(ord.output_asset, added_output);
        // Subtract budget + fee used to facilitate execution + adhoc fee.
        candidate.sub_asset(ord.fee_asset, consumed_budget + consumed_fee + proportional_fee);
        let consumed_bundle = Bundled(
            AdhocOrder(ord, adhoc_fee_input),
            FinalizedTxOut(consumed_utxo, in_ref),
        );
        let (residual_order, effect) = match result {
            Next::Term(_) => {
                candidate.null_datum();
                candidate.update_address(ord.redeemer_address.to_address(context.select::<NetworkId>()));
                (candidate, ExecutionEff::Eliminated(consumed_bundle))
            }
            // Adhoc orders must always be terminated.
            Next::Succ(AdhocOrder(_, _)) => {
                unreachable!()
            }
        };
        let witness = context.select::<DeployedValidator<{ InstantOrderWitnessV1 as u8 }>>();
        state.add_tx_fee(consumed_budget);
        state.add_operator_interest(consumed_fee + proportional_fee);
        state
            .tx_blueprint
            .add_witness(witness.erased(), PlutusData::new_list(vec![]));
        state.tx_blueprint.add_io(input, residual_order);
        state.tx_blueprint.add_ref_input(reference_utxo);
        (state, effect, context)
    }
}

impl<Ctx> BatchExec<ExecutionState, EffectPreview<GridOrder>, Ctx> for Magnet<Take<GridOrder, FinalizedTxOut>>
where
    Ctx: Has<NetworkId> + Has<DeployedValidator<{ GridOrderNative as u8 }>>,
{
    fn exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> (ExecutionState, EffectPreview<GridOrder>, Ctx) {
        let Magnet(trans) = self;
        trace!("Running transition: {}", trans);
        let removed_input = trans.removed_input();
        let added_output = trans.added_output();
        let consumed_budget = trans.consumed_budget();
        let consumed_fee = trans.consumed_fee();
        trace!(
            "GridOrder::exec(removed_input={}, added_output={}, consumed_budget={}, consumed_fee={})",
            removed_input,
            added_output,
            consumed_budget,
            consumed_fee
        );
        let Trans {
            target: Bundled(ord, FinalizedTxOut(consumed_out, in_ref)),
            result,
        } = trans;
        let DeployedValidatorErased {
            reference_utxo,
            hash,
            ex_budget,
            ..
        } = context
            .select::<DeployedValidator<{ GridOrderNative as u8 }>>()
            .erased();
        let input = ScriptInputBlueprint {
            reference: in_ref,
            utxo: consumed_out.clone(),
            script: ScriptWitness {
                hash,
                cost: ready_cost(ex_budget),
            },
            redeemer: ready_redeemer(limit::EXEC_REDEEMER),
            required_signers: vec![].into(),
        };
        let mut candidate = consumed_out.clone();
        let (input_asset, output_asset) = ord.absolute_io();
        // Subtract budget + fee used to facilitate execution.
        candidate.sub_asset(AssetClass::Native, consumed_budget + consumed_fee);
        // Subtract tradable input used in exchange.
        candidate.sub_asset(input_asset, removed_input);
        // Add output resulted from exchange.
        candidate.add_asset(output_asset, added_output);
        let consumed_bundle = Bundled(ord, FinalizedTxOut(consumed_out, in_ref));
        let (residual_order, effect) = match result {
            Next::Succ(next) => {
                if let Some(data) = candidate.data_mut() {
                    grid::unsafe_update_datum(data, next.quote_offer, next.price, next.side);
                }
                (
                    candidate.clone(),
                    ExecutionEff::Updated(consumed_bundle, Bundled(next, candidate)),
                )
            }
            Next::Term(_) => {
                candidate.null_datum();
                candidate.update_address(ord.redeemer_address.to_address(context.select::<NetworkId>()));
                (candidate, ExecutionEff::Eliminated(consumed_bundle))
            }
        };
        state.add_tx_fee(consumed_budget);
        state.add_operator_interest(consumed_fee);
        state.tx_blueprint.add_io(input, residual_order);
        state.tx_blueprint.add_ref_input(reference_utxo);
        (state, effect, context)
    }
}

/// Batch execution routing for [AnyPool].
impl<Ctx> BatchExec<ExecutionState, EffectPreview<AnyPool>, Ctx> for Magnet<Make<AnyPool, FinalizedTxOut>>
where
    Ctx: Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1LedgerFixed as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2 as u8 }>>,
{
    fn exec(self, state: ExecutionState, context: Ctx) -> (ExecutionState, EffectPreview<AnyPool>, Ctx) {
        match self.0 {
            Trans {
                target: Bundled(AnyPool::PureCFMM(p), src),
                result: Next::Succ(AnyPool::PureCFMM(p2)),
            } => {
                let (st, res, ctx) = Magnet(Trans {
                    target: Bundled(p, src),
                    result: Next::Succ(p2),
                })
                .exec(state, context);
                (
                    st,
                    res.bimap(|c| c.map(AnyPool::PureCFMM), |p| p.map(AnyPool::PureCFMM)),
                    ctx,
                )
            }
            Trans {
                target: Bundled(AnyPool::BalancedCFMM(p), src),
                result: Next::Succ(AnyPool::BalancedCFMM(p2)),
            } => {
                let (st, res, ctx) = Magnet(Trans {
                    target: Bundled(p, src),
                    result: Next::Succ(p2),
                })
                .exec(state, context);
                (
                    st,
                    res.bimap(|c| c.map(AnyPool::BalancedCFMM), |p| p.map(AnyPool::BalancedCFMM)),
                    ctx,
                )
            }
            Trans {
                target: Bundled(AnyPool::StableCFMM(p), src),
                result: Next::Succ(AnyPool::StableCFMM(p2)),
            } => {
                let (st, res, ctx) = Magnet(Trans {
                    target: Bundled(p, src),
                    result: Next::Succ(p2),
                })
                .exec(state, context);
                (
                    st,
                    res.bimap(|c| c.map(AnyPool::StableCFMM), |p| p.map(AnyPool::StableCFMM)),
                    ctx,
                )
            }
            _ => unreachable!(),
        }
    }
}

/// Batch execution logic for [ConstFnPool].
impl<Ctx> BatchExec<ExecutionState, EffectPreview<ConstFnPool>, Ctx>
    for Magnet<Make<ConstFnPool, FinalizedTxOut>>
where
    Ctx: Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1LedgerFixed as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2 as u8 }>>,
{
    fn exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> (ExecutionState, EffectPreview<ConstFnPool>, Ctx) {
        let Magnet(trans) = self;
        let side = trans.trade_side().expect("Empty swaps aren't allowed");
        let removed_liquidity = trans.loss().expect("Something must be removed");
        let added_liquidity = trans.gain().expect("Something must be added");
        let Trans {
            target: Bundled(pool, FinalizedTxOut(consumed_out, in_ref)),
            result,
        } = trans;
        let mut produced_out = consumed_out.clone();
        let PoolAssetMapping {
            asset_to_deduct_from,
            asset_to_add_to,
        } = pool.asset_mapping(side);
        trace!("ConstFnPool::exec(side={}, removed_liq={}, added_liq={}, asset_to_deduct_from={}, asset_to_add_to={})", side, removed_liquidity, added_liquidity, asset_to_deduct_from, asset_to_add_to);
        produced_out.sub_asset(asset_to_deduct_from, removed_liquidity);
        produced_out.add_asset(asset_to_add_to, added_liquidity);

        let DeployedValidatorErased {
            reference_utxo,
            hash,
            ex_budget,
            marginal_cost,
        } = pool.get_validator(&context);
        let input = ScriptInputBlueprint {
            reference: in_ref,
            utxo: consumed_out.clone(),
            script: ScriptWitness {
                hash,
                cost: delayed_cost(move |ctx| ex_budget + marginal_cost.scale(ctx.self_index as u64)),
            },
            redeemer: delayed_redeemer(move |ordering| {
                CFMMPoolRedeemer {
                    pool_input_index: ordering.index_of(&in_ref) as u64,
                    action: CFMMPoolAction::Swap,
                }
                .to_plutus_data()
            }),
            required_signers: vec![].into(),
        };

        let Next::Succ(transition) = result else {
            panic!("ConstFn pool isn't supposed to terminate in result of a trade")
        };

        if let Some(data) = produced_out.data_mut() {
            transition.unsafe_datum_update(data);
        }

        let updated_output = produced_out.clone();

        let consumed = Bundled(pool, FinalizedTxOut(consumed_out, in_ref));
        let produced = Bundled(transition, updated_output.clone());
        let trans = ExecutionEff::Updated(consumed, produced);

        state.tx_blueprint.add_io(input, updated_output);
        state.tx_blueprint.add_ref_input(reference_utxo);
        (state, trans, context)
    }
}

impl<Ctx> BatchExec<ExecutionState, EffectPreview<BalancePool>, Ctx>
    for Magnet<Make<BalancePool, FinalizedTxOut>>
where
    Ctx: Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>,
    Ctx: Has<DeployedValidator<{ BalanceFnPoolV2 as u8 }>>,
{
    fn exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> (ExecutionState, EffectPreview<BalancePool>, Ctx) {
        let Magnet(trans) = self;
        let side = trans.trade_side().expect("Empty swaps aren't allowed");
        let removed_liquidity = trans.loss().expect("Something must be removed");
        let added_liquidity = trans.gain().expect("Something must be added");
        let Trans {
            target: Bundled(pool, FinalizedTxOut(consumed_out, in_ref)),
            result,
        } = trans;
        let mut produced_out = consumed_out.clone();
        let PoolAssetMapping {
            asset_to_deduct_from,
            asset_to_add_to,
        } = pool.get_asset_deltas(side);
        produced_out.sub_asset(asset_to_deduct_from, removed_liquidity);
        produced_out.add_asset(asset_to_add_to, added_liquidity);

        let Next::Succ(transition) = result else {
            panic!("Balance pool isn't supposed to terminate in result of a trade")
        };

        let DeployedValidatorErased {
            reference_utxo,
            hash,
            ex_budget,
            marginal_cost,
        } = pool.get_validator(&context);
        let input = ScriptInputBlueprint {
            reference: in_ref,
            utxo: consumed_out.clone(),
            script: ScriptWitness {
                hash,
                cost: delayed_cost(move |ctx| ex_budget + marginal_cost.scale(ctx.self_index as u64)),
            },
            redeemer: delayed_redeemer(move |ordering| {
                BalancePoolRedeemer {
                    pool_input_index: ordering.index_of(&in_ref) as u64,
                    action: CFMMPoolAction::Swap,
                    new_pool_state: transition,
                    prev_pool_state: pool,
                }
                .to_plutus_data()
            }),
            required_signers: vec![].into(),
        };

        if let Some(data) = produced_out.data_mut() {
            balance_pool::unsafe_update_datum(
                data,
                transition.treasury_x.untag(),
                transition.treasury_y.untag(),
            );
        }

        let consumed = Bundled(pool, FinalizedTxOut(consumed_out, in_ref));
        let produced = Bundled(transition, produced_out.clone());
        let effect = ExecutionEff::Updated(consumed, produced);

        state.tx_blueprint.add_io(input, produced_out);
        state.tx_blueprint.add_ref_input(reference_utxo);
        (state, effect, context)
    }
}

impl<Ctx> BatchExec<ExecutionState, EffectPreview<StablePoolT2T>, Ctx>
    for Magnet<Make<StablePoolT2T, FinalizedTxOut>>
where
    Ctx: Has<DeployedValidator<{ StableFnPoolT2T as u8 }>>,
{
    fn exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> (ExecutionState, EffectPreview<StablePoolT2T>, Ctx) {
        let Magnet(trans) = self;
        let side = trans.trade_side().expect("Empty swaps aren't allowed");
        let removed_liquidity = trans.loss().expect("Something must be removed");
        let added_liquidity = trans.gain().expect("Something must be added");
        let Trans {
            target: Bundled(pool, FinalizedTxOut(consumed_out, in_ref)),
            result,
        } = trans;
        let mut produced_out = consumed_out.clone();
        let PoolAssetMapping {
            asset_to_deduct_from,
            asset_to_add_to,
        } = pool.get_asset_deltas(side);
        produced_out.sub_asset(asset_to_deduct_from, removed_liquidity);
        produced_out.add_asset(asset_to_add_to, added_liquidity);

        let Next::Succ(transition) = result else {
            panic!("Stable pool isn't supposed to terminate in result of a trade")
        };

        let DeployedValidatorErased {
            reference_utxo,
            hash,
            ex_budget,
            marginal_cost,
        } = pool.get_validator(&context);
        let input = ScriptInputBlueprint {
            reference: in_ref,
            utxo: consumed_out.clone(),
            script: ScriptWitness {
                hash,
                cost: delayed_cost(move |ctx| ex_budget + marginal_cost.scale(ctx.self_index as u64)),
            },
            redeemer: delayed_redeemer(move |ordering| {
                let pool_index = ordering.index_of(&in_ref) as u64;
                StablePoolRedeemer {
                    pool_input_index: pool_index,
                    pool_output_index: pool_index,
                    action: CFMMPoolAction::Swap,
                    new_pool_state: transition,
                    prev_pool_state: pool,
                }
                .to_plutus_data()
            }),
            required_signers: vec![].into(),
        };

        if let Some(data) = produced_out.data_mut() {
            stable_pool_t2t::unsafe_update_datum(
                data,
                transition.treasury_x.untag(),
                transition.treasury_y.untag(),
            );
        }

        let consumed = Bundled(pool, FinalizedTxOut(consumed_out, in_ref));
        let produced = Bundled(transition, produced_out.clone());
        let effect = ExecutionEff::Updated(consumed, produced);

        state.tx_blueprint.add_io(input, produced_out);
        state.tx_blueprint.add_ref_input(reference_utxo);
        (state, effect, context)
    }
}

impl<Ctx> BatchExec<ExecutionState, EffectPreview<QuadraticPool>, Ctx>
    for Magnet<Make<QuadraticPool, FinalizedTxOut>>
where
    Ctx: Has<DeployedValidator<{ DegenQuadraticPoolV1 as u8 }>>,
    Ctx: Has<DeployedValidator<{ DegenQuadraticPoolV1T2T as u8 }>>,
{
    fn exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> (ExecutionState, EffectPreview<QuadraticPool>, Ctx) {
        let Magnet(trans) = self;
        let side = trans.trade_side().expect("Empty swaps aren't allowed");
        let removed_liquidity = trans.loss().expect("Something must be removed");
        let added_liquidity = trans.gain().expect("Something must be added");
        let Trans {
            target: Bundled(pool, FinalizedTxOut(consumed_out, in_ref)),
            result,
        } = trans;
        let mut produced_out = consumed_out.clone();
        let PoolAssetMapping {
            asset_to_deduct_from,
            asset_to_add_to,
        } = pool.asset_mapping(side);
        produced_out.sub_asset(asset_to_deduct_from, removed_liquidity);
        produced_out.add_asset(asset_to_add_to, added_liquidity);

        let Next::Succ(transition) = result else {
            panic!("Degen pool isn't supposed to terminate in result of a trade")
        };

        if transition.ver == V1T2T {
            if let Some(data) = produced_out.data_mut() {
                quadratic_pool::unsafe_update_t2t_pd(data, transition.accumulated_x_fee);
            }
        }

        let DeployedValidatorErased {
            reference_utxo,
            hash,
            ex_budget,
            marginal_cost,
        } = pool.get_validator(&context);
        let input = ScriptInputBlueprint {
            reference: in_ref,
            utxo: consumed_out.clone(),
            script: ScriptWitness {
                hash,
                cost: delayed_cost(move |ctx| ex_budget + marginal_cost.scale(ctx.self_index as u64)),
            },
            redeemer: delayed_redeemer(move |ordering| {
                let pool_index = ordering.index_of(&in_ref) as u64;
                QuadraticPoolRedeemer {
                    pool_input_index: pool_index,
                    pool_output_index: pool_index,
                    action: CFMMPoolAction::Swap,
                }
                .to_plutus_data()
            }),
            required_signers: vec![].into(),
        };

        let consumed = Bundled(pool, FinalizedTxOut(consumed_out, in_ref));
        let produced = Bundled(transition, produced_out.clone());
        let effect = ExecutionEff::Updated(consumed, produced);

        state.tx_blueprint.add_io(input, produced_out);
        state.tx_blueprint.add_ref_input(reference_utxo);
        (state, effect, context)
    }
}
