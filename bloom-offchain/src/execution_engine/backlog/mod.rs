use std::fmt::Display;

use log::info;

use spectrum_offchain::backlog::HotBacklog;
use spectrum_offchain::data::order::SpecializedOrder;
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{Baked, EntitySnapshot, Stable};
use spectrum_offchain::executor::{RunOrder, RunOrderError};

use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::storage::kv_store::KvStore;

/// Backlog style executor for non-trade operations like AMM deposits/redeems.
pub trait BacklogExecutor<Pl, Op, Ver, Txc, Bearer, Store> {
    fn attempt(&mut self, store: &Store) -> Option<(Txc, Bundled<Baked<Pl, Ver>, Bearer>)>;
}

pub trait BacklogStateWrite<Op> {
    fn update_order(&mut self, order: Op);
    fn remove_order(&mut self, order: Op);
}

pub struct BacklogImpl<Backlog, Ctx> {
    backlog: Backlog,
    context: Ctx,
}

impl<Pl, Op, Ver, Txc, Bearer, Backlog, Store, Ctx> BacklogExecutor<Pl, Op, Ver, Txc, Bearer, Store>
    for BacklogImpl<Backlog, Ctx>
where
    Pl: Stable<StableId = Ver>,
    Bundled<Pl, Bearer>: RunOrder<Bundled<Op, Bearer>, Ctx, Txc>,
    Op: SpecializedOrder<TPoolId = Pl::StableId>,
    Op::TOrderId: Display,
    Backlog: HotBacklog<Bundled<Op, Bearer>>,
    Store: KvStore<Pl::StableId, Bundled<Pl, Bearer>>,
    Ctx: Clone,
{
    fn attempt(&mut self, store: &Store) -> Option<(Txc, Bundled<Baked<Pl, Ver>, Bearer>)> {
        if let Some((pool, op)) = self
            .backlog
            .try_pop()
            .and_then(|op| store.get(op.get_pool_ref()).map(|pl| (pl, op)))
        {
            let op_ref = op.get_self_ref();
            match pool.try_run(op, self.context.clone()) {
                Ok((tx_candidate, Predicted(updated_pool))) => {
                    return Some((
                        tx_candidate,
                        updated_pool.map(|p| {
                            let id = p.stable_id();
                            Baked::new(p, id)
                        }),
                    ))
                }
                Err(RunOrderError::NonFatal(err, _) | RunOrderError::Fatal(err, _)) => {
                    info!("Order {} dropped due to error: {}", op_ref, err);
                }
            }
        }
        None
    }
}
