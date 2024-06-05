use crate::execution_engine::liquidity_book::side::Side;
use crate::execution_engine::liquidity_book::types::AbsolutePrice;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Bin {
    pub amount: Side<u64>,
    pub price: AbsolutePrice,
}