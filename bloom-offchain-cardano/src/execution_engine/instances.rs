use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::TransactionBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::RedeemerTag;
use void::Void;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::liquidity_book::recipe::{LinkedFill, LinkedSwap};
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use spectrum_cardano_lib::output::{FinalizedTxOut, IndexedTxOut};
use spectrum_cardano_lib::plutus_data::RequiresRedeemer;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::constants::POOL_EXECUTION_UNITS;
use spectrum_offchain_cardano::data::pool::{CFMMPool, CFMMPoolAction, CFMMPoolRefScriptOutput};
use spectrum_offchain_cardano::data::PoolVer;

use crate::orders::AnyOrder;
use crate::pools::AnyPool;

/// Magnet for local instances.
#[repr(transparent)]
pub struct Magnet<T>(pub T);

impl<Ctx> BatchExec<TransactionBuilder, Option<IndexedTxOut>, Ctx, Void>
    for Magnet<LinkedFill<AnyOrder, FinalizedTxOut>>
{
    fn try_exec(
        self,
        accumulator: TransactionBuilder,
        context: Ctx,
    ) -> Result<(TransactionBuilder, Option<IndexedTxOut>, Ctx), Void> {
        todo!()
    }
}

/// Batch execution routing for [AnyPool].
impl<Ctx> BatchExec<TransactionBuilder, IndexedTxOut, Ctx, Void>
    for Magnet<LinkedSwap<AnyPool, FinalizedTxOut>>
where
    Ctx: Has<CFMMPoolRefScriptOutput<1>> + Has<CFMMPoolRefScriptOutput<2>>,
{
    fn try_exec(
        self,
        tx_builder: TransactionBuilder,
        context: Ctx,
    ) -> Result<(TransactionBuilder, IndexedTxOut, Ctx), Void> {
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
            .try_exec(tx_builder, context),
        }
    }
}

/// Batch execution logic for [CFMMPool].
impl<Ctx> BatchExec<TransactionBuilder, IndexedTxOut, Ctx, Void>
    for Magnet<LinkedSwap<CFMMPool, FinalizedTxOut>>
where
    Ctx: Has<CFMMPoolRefScriptOutput<1>> + Has<CFMMPoolRefScriptOutput<2>>,
{
    fn try_exec(
        self,
        mut tx_builder: TransactionBuilder,
        context: Ctx,
    ) -> Result<(TransactionBuilder, IndexedTxOut, Ctx), Void> {
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
        let successor_ix = tx_builder.output_sizes().len();
        let pool_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(produced_out.script_hash().unwrap()),
            CFMMPool::redeemer(CFMMPoolAction::Swap),
        );
        let pool_in = SingleInputBuilder::new(in_ref.into(), consumed_out)
            .plutus_script_inline_datum(pool_script, Vec::new())
            .unwrap();
        tx_builder
            .add_output(SingleOutputBuilderResult::new(produced_out))
            .unwrap();
        let pool_ref_script = match pool.ver {
            PoolVer::V1 => context.get::<CFMMPoolRefScriptOutput<1>>().0,
            _ => context.get::<CFMMPoolRefScriptOutput<2>>().0,
        };
        tx_builder.add_reference_input(pool_ref_script);
        tx_builder.add_input(pool_in).unwrap();
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
            POOL_EXECUTION_UNITS,
        );
        Ok((tx_builder, IndexedTxOut(successor_ix, successor), context))
    }
}
