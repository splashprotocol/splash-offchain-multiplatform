use spectrum_cardano_lib::TaggedAmount;
use spectrum_offchain_cardano::data::pool::CFMMPool;
use spectrum_offchain_cardano::data::OnChain;

use crate::liquidity_book::types::{Base, BatcherFeePerQuote, InclusionCost, LPFee, Price, Quote, SourceId};
use crate::time::TimeBounds;

pub trait LiquidityFragment<T> {
    fn id(&self) -> SourceId;
    fn amount(&self) -> TaggedAmount<Base>;
    fn price(&self) -> Price;
    fn batcher_fee(&self) -> BatcherFeePerQuote;
    fn cost_hint(&self) -> InclusionCost;
    fn time_bounds(&self) -> TimeBounds<T>;
}

pub trait LiquidityPool {
    fn id(&self) -> SourceId;
    fn amount_base(&self) -> TaggedAmount<Base>;
    fn amount_quote(&self) -> TaggedAmount<Quote>;
    fn price_hint(&self) -> Price;
    fn cost_hint(&self) -> InclusionCost;
    fn lp_fee(&self) -> LPFee;
    fn real_ask_price(&self, amount: TaggedAmount<Base>) -> Price;
    fn real_bid_price(&self, amount: TaggedAmount<Base>) -> Price;
}

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub enum AnyFragment {}

impl<T> LiquidityFragment<T> for AnyFragment {
    fn id(&self) -> SourceId {
        todo!()
    }

    fn amount(&self) -> TaggedAmount<Base> {
        todo!()
    }

    fn price(&self) -> Price {
        todo!()
    }

    fn batcher_fee(&self) -> BatcherFeePerQuote {
        todo!()
    }

    fn cost_hint(&self) -> InclusionCost {
        todo!()
    }

    fn time_bounds(&self) -> TimeBounds<T> {
        todo!()
    }
}

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum AnyPool {
    CFMM(OnChain<CFMMPool>),
}

impl LiquidityPool for AnyPool {
    fn id(&self) -> SourceId {
        todo!()
    }

    fn amount_base(&self) -> TaggedAmount<Base> {
        todo!()
    }

    fn amount_quote(&self) -> TaggedAmount<Quote> {
        todo!()
    }

    fn price_hint(&self) -> Price {
        todo!()
    }

    fn cost_hint(&self) -> InclusionCost {
        todo!()
    }

    fn lp_fee(&self) -> LPFee {
        todo!()
    }

    fn real_ask_price(&self, amount: TaggedAmount<Base>) -> Price {
        todo!()
    }

    fn real_bid_price(&self, amount: TaggedAmount<Base>) -> Price {
        todo!()
    }
}

#[derive(Debug, Copy, Clone)]
pub enum OneSideLiquidity<T> {
    Bid(T),
    Ask(T),
}

impl<T> OneSideLiquidity<T> {
    pub fn any(&self) -> &T {
        match self {
            OneSideLiquidity::Bid(t) => t,
            OneSideLiquidity::Ask(t) => t,
        }
    }
}
