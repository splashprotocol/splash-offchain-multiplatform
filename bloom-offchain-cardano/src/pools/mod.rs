use cml_chain::PolicyId;
use cml_multi_era::babbage::BabbageTransactionOutput;

use spectrum_cardano_lib::{OutputRef, Token};
use spectrum_offchain::data::EntitySnapshot;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pool::CFMMPool;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum AnyPool {
    CFMM(CFMMPool),
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for AnyPool {
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        CFMMPool::try_from_ledger(repr, ctx).map(AnyPool::CFMM)
    }
}

impl EntitySnapshot for AnyPool {
    type Version = OutputRef;
    type StableId = PolicyId;
    fn version(&self) -> Self::Version {
        match self {
            AnyPool::CFMM(p) => p.state_ver.into(),
        }
    }
    fn stable_id(&self) -> Self::StableId {
        match self {
            AnyPool::CFMM(p) => Token::from(p.id).0,
        }
    }
}
