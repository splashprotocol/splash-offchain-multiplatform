use std::cmp::Ordering;
use std::ops::Mul;

use cml_core::Slot;
use cml_crypto::Ed25519KeyHash;
use num_rational::Ratio;

use bloom_offchain::execution_engine::liquidity_book::fragment::Fragment;
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{BasePrice, ExecutionCost, Price};
use spectrum_cardano_lib::AssetClass;

use crate::orders::{Stateful, TLBCompatibleState};

const APPROX_AUCTION_COST: ExecutionCost = 1000;
const PRICE_DECAY_DEN: u64 = 10000;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct AuctionOrder {
    pub input_asset: AssetClass,
    pub input_amount: u64,
    pub output_asset: AssetClass,
    pub output_amount: u64,
    /// Price of input asset in output asset.
    pub start_price: Price,
    pub start_time: Slot,
    pub step_len: u32,
    pub steps: u32,
    pub price_decay: u64,
    pub fee_per_quote: Ratio<u128>,
    pub redeemer: Ed25519KeyHash,
}

impl PartialOrd for Stateful<AuctionOrder, TLBCompatibleState> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(&other))
    }
}

impl Ord for Stateful<AuctionOrder, TLBCompatibleState> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.weight().cmp(&other.weight())
    }
}

impl Fragment for Stateful<AuctionOrder, TLBCompatibleState> {
    fn side(&self) -> SideM {
        self.state.side
    }

    fn input(&self) -> u64 {
        self.order.input_amount
    }

    fn price(&self) -> BasePrice {
        let current_span = (self.state.time_now - self.order.start_time) / (self.order.step_len as u64);
        let decay =
            Ratio::new(self.order.price_decay as u128, PRICE_DECAY_DEN as u128).pow(current_span as i32);
        let decay = match self.side() {
            SideM::Bid => decay.pow(-1),
            SideM::Ask => decay,
        };
        let base_price = BasePrice::from_price(self.side(), self.order.start_price);
        base_price * decay
    }

    fn weight(&self) -> u64 {
        let decay =
            Ratio::new(self.order.price_decay as u128, PRICE_DECAY_DEN as u128).pow(self.order.steps as i32);
        let terminal_price = self.order.start_price * decay;
        Ratio::new(self.order.input_amount as u128, 1)
            .mul(terminal_price)
            .mul(self.order.fee_per_quote)
            .to_integer() as u64
    }

    fn cost_hint(&self) -> ExecutionCost {
        APPROX_AUCTION_COST
    }

    fn time_bounds(&self) -> TimeBounds<u64> {
        let lower = self.order.start_time;
        let upper = self.order.start_time + (self.order.step_len * self.order.steps) as u64;
        TimeBounds::Within(lower, upper)
    }
}

#[cfg(test)]
mod tests {
    use cml_crypto::Ed25519KeyHash;
    use num_rational::Ratio;

    use bloom_offchain::execution_engine::liquidity_book::fragment::Fragment;
    use bloom_offchain::execution_engine::liquidity_book::side::SideM;
    use bloom_offchain::execution_engine::liquidity_book::types::BasePrice;
    use spectrum_cardano_lib::AssetClass;

    use crate::orders::auction::{AuctionOrder, PRICE_DECAY_DEN};
    use crate::orders::{Stateful, TLBCompatibleState};

    #[test]
    fn correct_price_decay_as_time_advances_ask() {
        correct_price_decay_as_time_advances(SideM::Ask)
    }

    #[test]
    fn correct_price_decay_as_time_advances_bid() {
        correct_price_decay_as_time_advances(SideM::Bid)
    }

    fn correct_price_decay_as_time_advances(side: SideM) {
        let o = AuctionOrder {
            input_asset: AssetClass::Native,
            input_amount: 10_000_000_000,
            output_asset: AssetClass::Native,
            output_amount: 0,
            start_price: Ratio::new(100, 1),
            start_time: 0,
            step_len: 1,
            steps: 10,
            price_decay: 9900, // reduction of 1pp with each step
            fee_per_quote: Ratio::new(10, 1),
            redeemer: Ed25519KeyHash::from([0u8; 28]),
        };
        let init_state = TLBCompatibleState { side, time_now: 0 };
        let term_state = TLBCompatibleState { side, time_now: 10 };
        let term_price =
            o.start_price * Ratio::new(o.price_decay as u128, PRICE_DECAY_DEN as u128).pow(o.steps as i32);
        assert_eq!(
            Stateful::new(o, init_state).price(),
            BasePrice::from_price(side, o.start_price)
        );
        assert_eq!(
            Stateful::new(o, term_state).price(),
            BasePrice::from_price(side, term_price)
        );
    }
}
