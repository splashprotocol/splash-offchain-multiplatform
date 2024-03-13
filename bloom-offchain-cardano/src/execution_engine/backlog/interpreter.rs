use std::fmt::Display;

use clap::ArgAction::Version;
use log::info;

use bloom_offchain::execution_engine::backlog::SpecializedInterpreter;
use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_offchain::data::order::SpecializedOrder;
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{Baked, EntitySnapshot, Stable};
use spectrum_offchain::executor::{RunOrder, RunOrderError};

use crate::pools::PoolMagnet;

#[derive(Debug, Copy, Clone)]
pub struct SpecializedInterpreterViaRunOrder;

impl<'a, Pl, Ord, Ver, Txc, Bearer, Ctx> SpecializedInterpreter<Pl, Ord, Ver, Txc, Bearer, Ctx>
    for SpecializedInterpreterViaRunOrder
where
    Pl: EntitySnapshot<Version = Ver>,
    PoolMagnet<Bundled<Pl, Bearer>>: RunOrder<Bundled<Ord, Bearer>, Ctx, Txc>,
    Bearer: Clone,
    Ord: SpecializedOrder<TPoolId = Pl::StableId> + Clone,
    Ord::TOrderId: Display,
    Ctx: Clone,
{
    fn try_run(
        &mut self,
        pool: Bundled<Pl, Bearer>,
        order: Bundled<Ord, Bearer>,
        context: Ctx,
    ) -> Option<(Txc, Bundled<Baked<Pl, Ver>, Bearer>, Bundled<Ord, Bearer>)> {
        let op_ref = order.get_self_ref();
        match PoolMagnet(pool).try_run(order.clone(), context.clone()) {
            Ok((tx_candidate, Predicted(updated_pool))) => {
                return Some((
                    tx_candidate,
                    updated_pool.0.map(|p| {
                        let ver = p.version();
                        Baked::new(p, ver)
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
