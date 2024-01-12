use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::{ConstrPlutusData, PlutusData, RedeemerTag};
use cml_chain::utils::BigInt;
use void::Void;

use crate::execution_engine::execution_state::ExecutionState;
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
use spectrum_offchain_cardano::data::pool::{CFMMPool, CFMMPoolAction, CFMMPoolRefScriptOutput};
use spectrum_offchain_cardano::data::PoolVer;

use crate::orders::auction::AUCTION_EXECUTION_UNITS;
use crate::orders::spot::SpotOrder;
use crate::orders::AnyOrder;
use crate::pools::AnyPool;

/// Magnet for local instances.
#[repr(transparent)]
pub struct Magnet<T>(pub T);

impl<Ctx> BatchExec<ExecutionState, Option<IndexedTxOut>, Ctx, Void>
    for Magnet<LinkedFill<AnyOrder, FinalizedTxOut>>
{
    fn try_exec(
        self,
        state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, Option<IndexedTxOut>, Ctx), Void> {
        match self.0 {
            LinkedFill {
                target: Bundled(AnyOrder::Spot(o), src),
                transition,
                removed_input,
                added_output,
            } => Magnet(LinkedFill {
                target: Bundled(o, src),
                transition: transition.map(|AnyOrder::Spot(o2)| o2),
                removed_input,
                added_output,
            })
            .try_exec(state, context),
        }
    }
}

impl<Ctx> BatchExec<ExecutionState, Option<IndexedTxOut>, Ctx, Void>
    for Magnet<LinkedFill<SpotOrder, FinalizedTxOut>>
{
    fn try_exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, Option<IndexedTxOut>, Ctx), Void> {
        let Magnet(LinkedFill {
            target: Bundled(ord, FinalizedTxOut(consumed_out, in_ref)),
            transition,
            removed_input,
            added_output,
        }) = self;
        let mut candidate = consumed_out.clone();
        candidate.sub_asset(ord.input_asset, removed_input);
        candidate.add_asset(ord.output_asset, added_output);
        let residual_order = match transition {
            StateTrans::Active(_) => Some(candidate.clone()),
            StateTrans::EOL => {
                candidate.null_datum();
                candidate.update_payment_cred(ord.redeemer_cred());
                None
            }
        };
        // todo: replace `tx_builder.output_sizes()`
        let successor_ix = state.tx_builder.output_sizes().len();
        let order_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(candidate.script_hash().unwrap()),
            spot_exec_redeemer(successor_ix as u16),
        );
        let order_in = SingleInputBuilder::new(in_ref.into(), consumed_out)
            .plutus_script_inline_datum(order_script, Vec::new())
            .unwrap();
        state
            .tx_builder
            .add_output(SingleOutputBuilderResult::new(candidate))
            .unwrap();
        state.tx_builder.add_input(order_in).unwrap();
        state.tx_builder.set_exunits(
            // todo: check for possible collisions bc of fixed 0-index.
            RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
            AUCTION_EXECUTION_UNITS,
        );
        let execution_fee = ord.fee_per_output.into_cml_value(added_output);
        state.executor_fee_acc.checked_add(&execution_fee).unwrap();
        Ok((
            state,
            residual_order.map(|o| IndexedTxOut(successor_ix, o)),
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
impl<Ctx> BatchExec<ExecutionState, IndexedTxOut, Ctx, Void> for Magnet<LinkedSwap<AnyPool, FinalizedTxOut>>
where
    Ctx: Has<CFMMPoolRefScriptOutput<1>> + Has<CFMMPoolRefScriptOutput<2>>,
{
    fn try_exec(
        self,
        state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, IndexedTxOut, Ctx), Void> {
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

/// Batch execution logic for [CFMMPool].
impl<Ctx> BatchExec<ExecutionState, IndexedTxOut, Ctx, Void> for Magnet<LinkedSwap<CFMMPool, FinalizedTxOut>>
where
    Ctx: Has<CFMMPoolRefScriptOutput<1>> + Has<CFMMPoolRefScriptOutput<2>>,
{
    fn try_exec(
        self,
        mut state: ExecutionState,
        context: Ctx,
    ) -> Result<(ExecutionState, IndexedTxOut, Ctx), Void> {
        let Magnet(LinkedSwap {
            target: Bundled(pool, FinalizedTxOut(consumed_out, in_ref)),
            side,
            input,
            output,
            ..
        }) = self;
        let mut produced_out = consumed_out.clone();
        let (removed_asset, added_asset) = match side {
            SideM::Bid => (pool.asset_x.untag(), pool.asset_y.untag()),
            SideM::Ask => (pool.asset_y.untag(), pool.asset_x.untag()),
        };
        produced_out.sub_asset(removed_asset, output);
        produced_out.add_asset(added_asset, input);
        let successor = produced_out.clone();
        let successor_ix = state.tx_builder.output_sizes().len();
        let pool_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(produced_out.script_hash().unwrap()),
            CFMMPool::redeemer(CFMMPoolAction::Swap),
        );
        let pool_in = SingleInputBuilder::new(in_ref.into(), consumed_out)
            .plutus_script_inline_datum(pool_script, Vec::new())
            .unwrap();
        state
            .tx_builder
            .add_output(SingleOutputBuilderResult::new(produced_out))
            .unwrap();
        let pool_ref_script = match pool.ver {
            PoolVer::V1 => context.get::<CFMMPoolRefScriptOutput<1>>().0,
            _ => context.get::<CFMMPoolRefScriptOutput<2>>().0,
        };
        state.tx_builder.add_reference_input(pool_ref_script);
        state.tx_builder.add_input(pool_in).unwrap();
        state.tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
            POOL_EXECUTION_UNITS,
        );
        Ok((state, IndexedTxOut(successor_ix, successor), context))
    }
}
