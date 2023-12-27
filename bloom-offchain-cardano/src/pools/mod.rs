use bloom_offchain::execution_engine::liquidity_book::pool::{Pool, PoolQuality};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::types::BasePrice;
use bloom_offchain::execution_engine::types::StableId;
use cml_chain::PolicyId;
use cml_multi_era::babbage::BabbageTransactionOutput;

use spectrum_cardano_lib::{OutputRef, Token};
use spectrum_offchain::data::{EntitySnapshot, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pool::CFMMPool;

use crate::PairId;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum AnyPool {
    CFMM(CFMMPool),
}

impl Pool for AnyPool {
    fn id(&self) -> StableId {
        todo!()
    }

    fn static_price(&self) -> BasePrice {
        todo!()
    }

    fn real_price(&self, input: Side<u64>) -> BasePrice {
        todo!()
    }

    fn swap(self, input: Side<u64>) -> (u64, Self) {
        todo!()
    }

    fn quality(&self) -> PoolQuality {
        todo!()
    }
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

impl Tradable for AnyPool {
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        match self {
            AnyPool::CFMM(p) => todo!(),
        }
    }
}
