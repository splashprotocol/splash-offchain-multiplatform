use cml_chain::{transaction::TransactionOutput, Value};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{transaction::TransactionOutputExtension, OutputRef};
use spectrum_offchain::{
    data::{Has, HasIdentifier, Identifier, Stable},
    ledger::TryFromLedger,
};

use crate::{entities::Snapshot, protocol_config::OperatorCreds};

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
    pub value: Value,
    pub id: FundingBoxId,
}

impl<C> TryFromLedger<TransactionOutput, C> for FundingBoxSnapshot
where
    C: Has<OperatorCreds> + Has<OutputRef>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        let OperatorCreds(_, addr) = ctx.select::<OperatorCreds>();
        if addr == *repr.address() {
            let value = repr.value().clone();
            let output_ref = ctx.select::<OutputRef>();
            return Some(Snapshot(
                FundingBox {
                    value,
                    id: FundingBoxId(output_ref),
                },
                output_ref,
            ));
        }
        None
    }
}
