use spectrum_offchain::backlog::HotBacklog;
use spectrum_offchain::data::EntitySnapshot;
use spectrum_offchain::data::order::SpecializedOrder;
use spectrum_offchain::executor::{RunOrder, RunOrderError};

use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::storage::kv_store::KvStore;

/// Backlog style executor for non-trade operations like AMM deposits/redeems.
pub trait BacklogExecutor<Pl, Op, Txc, Bearer> {
    fn attempt(&mut self) -> Option<(Txc, Bundled<Pl, Bearer>)>;
}

pub trait BacklogStateWrite<Pl, Op> {
    fn update_pool(&mut self, pool: Pl);
    fn remove_pool(&mut self, pool: Pl);
    fn update_order(&mut self, order: Op);
    fn remove_order(&mut self, order: Op);
}

pub struct BacklogImpl<Backlog, Store, Ctx> {
    backlog: Backlog,
    store: Store,
    context: Ctx,
}

impl<Pl, Op, Txc, Bearer, Backlog, Store, Ctx> BacklogExecutor<Pl, Op, Txc, Bearer>
    for BacklogImpl<Backlog, Store, Ctx>
where
    Pl: EntitySnapshot + RunOrder<Op, Ctx, Txc>,
    Op: SpecializedOrder<TPoolId = Pl::StableId>,
    Backlog: HotBacklog<Op>,
    Store: KvStore<Pl::StableId, Pl>,
    Ctx: Clone,
{
    fn attempt(&mut self) -> Option<(Txc, Bundled<Pl, Bearer>)> {
        if let Some((pool, op)) = self
            .backlog
            .try_pop()
            .and_then(|op| self.store.get(op.get_pool_ref()).map(|pl| (pl, op)))
        {
            match pool.try_run(op, self.context.clone()) {
                Ok((tx_candidate, updated_pool)) => {}
                Err(RunOrderError::NonFatal(err, _) | RunOrderError::Fatal(err, _)) => {}
            }
        }
        None
    }
}
