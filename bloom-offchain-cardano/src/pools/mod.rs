use bloom_offchain::execution_engine::liquidity_book::pool::{Pool, PoolQuality};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use cml_chain::PolicyId;
use cml_multi_era::babbage::BabbageTransactionOutput;

use spectrum_cardano_lib::{OutputRef, Token};
use spectrum_offchain::data::{EntitySnapshot, Has, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::data::pool::ClassicCFMMPool;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum AnyPool {
    CFMM(ClassicCFMMPool),
}

impl Pool for AnyPool {
    fn static_price(&self) -> AbsolutePrice {
        match self {
            AnyPool::CFMM(p) => p.static_price(),
        }
    }

    fn real_price(&self, input: Side<u64>) -> AbsolutePrice {
        match self {
            AnyPool::CFMM(p) => p.real_price(input),
        }
    }

    fn swap(self, input: Side<u64>) -> (u64, Self) {
        match self {
            AnyPool::CFMM(p) => {
                let (out, p2) = p.swap(input);
                (out, AnyPool::CFMM(p2))
            }
        }
    }

    fn quality(&self) -> PoolQuality {
        match self {
            AnyPool::CFMM(p) => p.quality(),
        }
    }
}

impl<C> TryFromLedger<BabbageTransactionOutput, C> for AnyPool
where
    C: Has<OutputRef>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: C) -> Option<Self> {
        ClassicCFMMPool::try_from_ledger(repr, ctx.get()).map(AnyPool::CFMM)
    }
}

impl Stable for AnyPool {
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        match self {
            AnyPool::CFMM(p) => Token::from(p.id).0,
        }
    }
}

impl EntitySnapshot for AnyPool {
    type Version = OutputRef;
    fn version(&self) -> Self::Version {
        match self {
            AnyPool::CFMM(p) => p.state_ver.into(),
        }
    }
}

impl Tradable for AnyPool {
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        match self {
            AnyPool::CFMM(p) => PairId::canonical(p.asset_x.untag(), p.asset_y.untag()),
        }
    }
}
