use std::fmt::Display;

use log::info;

use bloom_offchain::execution_engine::backlog::SpecializedInterpreter;
use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::event::Predicted;
use spectrum_offchain::domain::order::SpecializedOrder;
use spectrum_offchain::domain::{Baked, Stable};
use spectrum_offchain::executor::{RunOrder, RunOrderError};

use crate::pools::PoolMagnet;

#[derive(Debug, Copy, Clone)]
pub struct SpecializedInterpreterViaRunOrder;

impl<'a, Pl, Ord, Ver, Txc, Ctx> SpecializedInterpreter<Pl, Ord, Ver, Txc, FinalizedTxOut, Ctx>
    for SpecializedInterpreterViaRunOrder
where
    Pl: Stable,
    PoolMagnet<Bundled<Pl, FinalizedTxOut>>: RunOrder<Bundled<Ord, FinalizedTxOut>, Ctx, Txc>,
    Ord: SpecializedOrder<TPoolId = Pl::StableId> + Clone,
    Ord::TOrderId: Display,
    Ver: From<OutputRef>,
    Ctx: Clone,
{
    fn try_run(
        &mut self,
        pool: Bundled<Pl, FinalizedTxOut>,
        order: Bundled<Ord, FinalizedTxOut>,
        context: Ctx,
    ) -> Option<(
        Txc,
        Bundled<Baked<Pl, Ver>, FinalizedTxOut>,
        Bundled<Ord, FinalizedTxOut>,
    )> {
        let op_ref = order.get_self_ref();
        match PoolMagnet(pool).try_run(order.clone(), context.clone()) {
            Ok((tx_candidate, Predicted(PoolMagnet(Bundled(pool, bearer))))) => {
                return Some((
                    tx_candidate,
                    Bundled(Baked::new(pool, bearer.1.into()), bearer),
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
