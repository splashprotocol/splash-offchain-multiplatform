use cml_chain::{transaction::TransactionOutput, Value};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{transaction::TransactionOutputExtension, OutputRef};
use spectrum_offchain::{
    domain::{Has, Stable},
    ledger::{IntoLedger, TryFromLedger},
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
    derive_more::Into,
    Serialize,
    Deserialize,
    Hash,
    Debug,
    derive_more::Display,
)]
pub struct FundingBoxId(OutputRef);

impl Stable for FundingBox {
    type StableId = FundingBoxId;

    fn stable_id(&self) -> Self::StableId {
        self.id
    }

    fn is_quasi_permanent(&self) -> bool {
        false
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
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
            let ver = ctx.select::<OutputRef>();
            return Some(Snapshot(
                FundingBox {
                    value,
                    id: FundingBoxId(ver),
                },
                ver,
            ));
        }
        None
    }
}

impl<Ctx> IntoLedger<TransactionOutput, Ctx> for FundingBox
where
    Ctx: Has<OperatorCreds>,
{
    fn into_ledger(self, ctx: Ctx) -> TransactionOutput {
        let OperatorCreds(_, address) = ctx.select::<OperatorCreds>();
        TransactionOutput::new(address, self.value, None, None)
    }
}
