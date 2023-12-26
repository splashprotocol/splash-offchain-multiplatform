use cml_chain::PolicyId;
use cml_multi_era::babbage::BabbageTransactionOutput;
use futures::future::Either;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::{EntitySnapshot, Tradable};
use spectrum_offchain::ledger::TryFromLedger;

use crate::orders::AnyOrder;
use crate::pools::AnyPool;
use crate::PairId;

pub mod entity_index;
pub mod handler;

#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct CardanoEvent(Bundled<Either<AnyOrder, AnyPool>, FinalizedTxOut>);

impl EntitySnapshot for CardanoEvent {
    type Version = OutputRef;
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

impl Tradable for CardanoEvent {
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        todo!()
    }
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for CardanoEvent {
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        <Either<AnyOrder, AnyPool>>::try_from_ledger(repr, ctx)
            .map(|inner| Self(Bundled(inner, FinalizedTxOut::new(repr.clone(), ctx))))
    }
}
