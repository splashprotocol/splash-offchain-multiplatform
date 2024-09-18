use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::{HasIdentifier, Identifier, Stable};

use crate::entities::Snapshot;

pub type FundingBoxSnapshot = Snapshot<FundingBox, OutputRef>;

#[derive(
    Copy,
    Clone,
    PartialEq,
    Eq,
    Ord,
    PartialOrd,
    derive_more::From,
    Serialize,
    Deserialize,
    Hash,
    Debug,
    derive_more::Display,
)]
pub struct FundingBoxId(OutputRef);

impl Identifier for FundingBoxId {
    type For = FundingBoxSnapshot;
}

impl HasIdentifier for FundingBoxSnapshot {
    type Id = FundingBoxId;

    fn identifier(&self) -> Self::Id {
        FundingBoxId(self.1)
    }
}

impl Stable for FundingBox {
    type StableId = FundingBoxId;

    fn stable_id(&self) -> Self::StableId {
        self.id
    }

    fn is_quasi_permanent(&self) -> bool {
        false
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FundingBox {
    pub lovelaces: u64,
    pub id: FundingBoxId,
}
