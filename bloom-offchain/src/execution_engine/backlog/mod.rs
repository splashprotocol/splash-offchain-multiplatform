use std::fmt::Display;

use log::info;

use spectrum_offchain::data::{Baked, Stable};
use spectrum_offchain::data::order::SpecializedOrder;
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::executor::{RunOrder, RunOrderError};

use crate::execution_engine::bundled::Bundled;

/// Interpreter for non-trade operations like AMM deposits/redeems.
pub trait SpecializedInterpreter<Pl, Op, Ver, Txc, Bearer, Ctx> {
    fn try_run(
        &mut self,
        pool: Bundled<Pl, Bearer>,
        order: Bundled<Op, Bearer>,
        context: Ctx,
    ) -> Option<(Txc, Bundled<Baked<Pl, Ver>, Bearer>, Bundled<Op, Bearer>)>;
}

pub struct SpecializedInterpreterImpl;

impl<'a, Pl, Op, Ver, Txc, Bearer, Ctx> SpecializedInterpreter<Pl, Op, Ver, Txc, Bearer, Ctx>
    for SpecializedInterpreterImpl
where
    Pl: Stable<StableId = Ver>,
    Bundled<Pl, Bearer>: RunOrder<Bundled<Op, Bearer>, Ctx, Txc>,
    Bearer: Clone,
    Op: SpecializedOrder<TPoolId = Pl::StableId> + Clone,
    Op::TOrderId: Display,
    Ctx: Clone,
{
    fn try_run(
        &mut self,
        pool: Bundled<Pl, Bearer>,
        order: Bundled<Op, Bearer>,
        context: Ctx,
    ) -> Option<(Txc, Bundled<Baked<Pl, Ver>, Bearer>, Bundled<Op, Bearer>)> {
        let op_ref = order.get_self_ref();
        match pool.try_run(order.clone(), context.clone()) {
            Ok((tx_candidate, Predicted(updated_pool))) => {
                return Some((
                    tx_candidate,
                    updated_pool.map(|p| {
                        let id = p.stable_id();
                        Baked::new(p, id)
                    }),
                    order,
                ))
            }
            Err(RunOrderError::NonFatal(err, _) | RunOrderError::Fatal(err, _)) => {
                info!("Order {} dropped due to error: {}", op_ref, err);
            }
        }
        None
    }
}
