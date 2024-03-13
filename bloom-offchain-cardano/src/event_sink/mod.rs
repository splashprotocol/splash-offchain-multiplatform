use cml_chain::PolicyId;
use cml_multi_era::babbage::BabbageTransactionOutput;
use either::Either;
use log::trace;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::{Baked, EntitySnapshot, Has, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pair::PairId;

use crate::creds::ExecutorCred;
use crate::orders::AnyOrder;
use crate::pools::AnyPool;

pub mod entity_index;
pub mod handler;

#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct EvolvingCardanoEntity(
    pub Bundled<Either<Baked<AnyOrder, OutputRef>, Baked<AnyPool, OutputRef>>, FinalizedTxOut>,
);

impl Stable for EvolvingCardanoEntity {
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
}

impl EntitySnapshot for EvolvingCardanoEntity {
    type Version = OutputRef;
    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

impl Tradable for EvolvingCardanoEntity {
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        self.0.pair_id()
    }
}

impl<C> TryFromLedger<BabbageTransactionOutput, C> for EvolvingCardanoEntity
where
    C: Copy + Has<ExecutorCred> + Has<OutputRef>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: C) -> Option<Self> {
        trace!(target: "offchain", "CardanoEntity::try_from_ledger");
        <Either<Baked<AnyOrder, OutputRef>, Baked<AnyPool, OutputRef>>>::try_from_ledger(repr, ctx).map(
            |inner| {
                Self(Bundled(
                    inner,
                    FinalizedTxOut::new(repr.clone(), ctx.get_labeled::<OutputRef>()),
                ))
            },
        )
    }
}
