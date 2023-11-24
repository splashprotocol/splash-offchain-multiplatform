use crate::liquidity_book::side::SideMarker;
use crate::liquidity_book::types::{LPFee, Price};

#[derive(Debug, Copy, Clone)]
pub enum Pool {}

impl Pool {
    pub fn price_hint(&self) -> Price {
        todo!()
    }
    pub fn lp_fee(&self) -> LPFee {
        todo!()
    }
    pub fn real_price(&self, side: SideMarker, amount: u64) -> Price {
        todo!()
    }
}
