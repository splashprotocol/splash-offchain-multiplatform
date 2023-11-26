use spectrum_offchain_cardano::data::pool::CFMMPool;

use crate::liquidity_book::side::SideMarker;
use crate::liquidity_book::types::{LPFee, Price};

#[derive(Debug, Copy, Clone)]
pub enum Pool {
    CFMM(CFMMPool)
}

impl Pool {
    pub fn price_hint(&self) -> Price {
        todo!()
    }
    pub fn lp_fee(&self) -> LPFee {
        todo!()
    }
    pub fn real_price(&self, side: SideMarker, input: u64) -> Price {
        todo!()
    }
    pub fn output(&self, side: SideMarker, input: u64) -> u64 {
        todo!()
    }
}
