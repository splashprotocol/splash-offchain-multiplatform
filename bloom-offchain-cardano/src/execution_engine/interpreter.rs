use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionBuilder};
use tailcall::tailcall;
use void::Void;

use bloom_offchain::execution_engine::exec::BatchExec;
use bloom_offchain::execution_engine::interpreter::RecipeInterpreter;
use bloom_offchain::execution_engine::liquidity_book::recipe::{
    ExecutionRecipe, Fill, PartialFill, Swap, TerminalInstruction,
};
use bloom_offchain::execution_engine::source_db::SourceDB;
use bloom_offchain::execution_engine::StableId;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::{FinalizedTxOut, IndexedTxOut};
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::Has;

use crate::execution_engine::bundled::Bundled;
use crate::operator_address::OperatorAddress;

/// A short-living interpreter.
pub struct CardanoRecipeInterpreter<'a, SrcDB> {
    sources: &'a mut SrcDB,
}

impl<'a, Fr, Pl, SrcDB, Ctx> RecipeInterpreter<Fr, Pl, Ctx, FinalizedTxOut, SignedTxBuilder>
    for CardanoRecipeInterpreter<'a, SrcDB>
where
    Fr: Has<StableId>,
    Pl: Has<StableId>,
    Bundled<Fill<Fr>, FinalizedTxOut>: BatchExec<TransactionBuilder, Option<IndexedTxOut>, Ctx, Void>,
    Bundled<PartialFill<Fr>, FinalizedTxOut>: BatchExec<TransactionBuilder, Option<IndexedTxOut>, Ctx, Void>,
    Bundled<Swap<Pl>, FinalizedTxOut>: BatchExec<TransactionBuilder, IndexedTxOut, Ctx, Void>,
    SrcDB: SourceDB<StableId, FinalizedTxOut>,
    Ctx: Clone + Has<Collateral> + Has<OperatorAddress>,
{
    fn run(
        &mut self,
        ExecutionRecipe(instructions): ExecutionRecipe<Fr, Pl>,
        ctx: Ctx,
    ) -> (SignedTxBuilder, Vec<(StableId, FinalizedTxOut)>) {
        let tx_builder = constant_tx_builder();
        let (mut tx_builder, mut indexed_outputs, ctx) =
            execute(self.sources, ctx, tx_builder, vec![], instructions);
        tx_builder.add_collateral(ctx.get::<Collateral>().into()).unwrap();
        let tx = tx_builder
            .build(ChangeSelectionAlgo::Default, &ctx.get::<OperatorAddress>().into())
            .unwrap();
        let mut finalized_outputs = vec![];
        let tx_hash = hash_transaction_canonical(&tx.body());
        while let Some((sid, IndexedTxOut(ix, out))) = indexed_outputs.pop() {
            let out_ref = OutputRef::new(tx_hash, ix as u64);
            let finalized_out = FinalizedTxOut(out, out_ref);
            finalized_outputs.push((sid, finalized_out))
        }
        (tx, finalized_outputs)
    }
}

#[tailcall]
fn execute<Fr, Pl, SrcDB, Ctx>(
    sources: &mut SrcDB,
    ctx: Ctx,
    tx_builder: TransactionBuilder,
    mut updates_acc: Vec<(StableId, IndexedTxOut)>,
    mut rem: Vec<TerminalInstruction<Fr, Pl>>,
) -> (TransactionBuilder, Vec<(StableId, IndexedTxOut)>, Ctx)
where
    Fr: Has<StableId>,
    Pl: Has<StableId>,
    Bundled<Fill<Fr>, FinalizedTxOut>: BatchExec<TransactionBuilder, Option<IndexedTxOut>, Ctx, Void>,
    Bundled<PartialFill<Fr>, FinalizedTxOut>: BatchExec<TransactionBuilder, Option<IndexedTxOut>, Ctx, Void>,
    Bundled<Swap<Pl>, FinalizedTxOut>: BatchExec<TransactionBuilder, IndexedTxOut, Ctx, Void>,
    SrcDB: SourceDB<StableId, FinalizedTxOut>,
    Ctx: Clone,
{
    if let Some(instruction) = rem.pop() {
        match instruction {
            TerminalInstruction::Fill(fill_order) => {
                let sid = fill_order.target.get::<StableId>();
                let bundled = Bundled(fill_order, sources.take_unsafe(sid));
                let (tx_builder, next, ctx) = bundled.try_exec(tx_builder, ctx).unwrap();
                if let Some(residue) = next {
                    updates_acc.push((sid, residue));
                }
                execute(sources, ctx, tx_builder, updates_acc, rem)
            }
            TerminalInstruction::Swap(swap) => {
                let sid = swap.target.get::<StableId>();
                let bundled = Bundled(swap, sources.take_unsafe(sid));
                let (tx_builder, next, ctx) = bundled.try_exec(tx_builder, ctx).unwrap();
                updates_acc.push((sid, next));
                execute(sources, ctx, tx_builder, updates_acc, rem)
            }
        }
    } else {
        return (tx_builder, updates_acc, ctx);
    }
}
