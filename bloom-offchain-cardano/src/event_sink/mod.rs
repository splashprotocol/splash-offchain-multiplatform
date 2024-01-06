use cml_chain::PolicyId;
use cml_multi_era::babbage::BabbageTransactionOutput;
use either::Either;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::{Baked, EntitySnapshot, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pair::PairId;

use crate::orders::AnyOrder;
use crate::pools::AnyPool;

pub mod entity_index;
pub mod handler;

#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct CardanoEntity(
    pub Bundled<Either<Baked<AnyOrder, OutputRef>, Baked<AnyPool, OutputRef>>, FinalizedTxOut>,
);

impl Stable for CardanoEntity {
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
}

impl EntitySnapshot for CardanoEntity {
    type Version = OutputRef;
    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

impl Tradable for CardanoEntity {
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        self.0.pair_id()
    }
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for CardanoEntity {
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        <Either<Baked<AnyOrder, OutputRef>, Baked<AnyPool, OutputRef>>>::try_from_ledger(repr, ctx)
            .map(|inner| Self(Bundled(inner, FinalizedTxOut::new(repr.clone(), ctx))))
    }
}
