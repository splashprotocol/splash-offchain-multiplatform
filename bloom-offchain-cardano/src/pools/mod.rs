use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::PolicyId;
use cml_multi_era::babbage::BabbageTransactionOutput;

use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::liquidity_book::pool::{Pool, PoolQuality};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::{OutputRef, Token};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{EntitySnapshot, Has, Stable, Tradable};
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::order::ClassicalAMMOrder;
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::data::pool::CFMMPool;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum AnyPool {
    CFMM(CFMMPool),
}

/// Magnet for local instances.
#[repr(transparent)]
pub struct PoolMagnet<T>(pub T);

impl<Ctx> RunOrder<Bundled<ClassicalAMMOrder, FinalizedTxOut>, Ctx, SignedTxBuilder>
    for PoolMagnet<Bundled<AnyPool, FinalizedTxOut>>
where
    spectrum_offchain_cardano::data::execution_context::ExecutionContext: From<Ctx>,
{
    fn try_run(
        self,
        order: Bundled<ClassicalAMMOrder, FinalizedTxOut>,
        ctx: Ctx,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<Bundled<ClassicalAMMOrder, FinalizedTxOut>>>
    {
        let PoolMagnet(Bundled(pool, bearer)) = self;
        match pool {
            AnyPool::CFMM(cfmm_pool) => Bundled(cfmm_pool, bearer)
                .try_run(order, ctx.into())
                .map(|(txb, Predicted(bundle))| (txb, Predicted(PoolMagnet(bundle.map(AnyPool::CFMM))))),
        }
    }
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
        CFMMPool::try_from_ledger(repr, ctx.get()).map(AnyPool::CFMM)
    }
}

impl Stable for AnyPool {
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        match self {
            AnyPool::CFMM(p) => Token::from(p.id).0,
        }
    }
    fn is_quasi_permanent(&self) -> bool {
        true
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
