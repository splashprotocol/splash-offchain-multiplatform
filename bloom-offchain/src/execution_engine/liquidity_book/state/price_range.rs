use crate::execution_engine::liquidity_book::market_taker::MarketTaker;
use crate::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_offchain::display::display_option;
use std::fmt::{Display, Formatter};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct AllowedPriceRange {
    pub max_ask_price: Option<AbsolutePrice>,
    pub min_bid_price: Option<AbsolutePrice>,
}

impl AllowedPriceRange {
    /// Check if given `ask` falls within the price range.
    pub fn test_ask<T: MarketTaker>(&self, ask: T) -> Option<T> {
        match self.max_ask_price {
            None => Some(ask),
            Some(max_ask) => {
                if ask.price() <= max_ask {
                    Some(ask)
                } else {
                    None
                }
            }
        }
    }

    /// Check if given `bid` falls within the price range.
    pub fn test_bid<T: MarketTaker>(&self, bid: T) -> Option<T> {
        match self.max_ask_price {
            None => Some(bid),
            Some(min_bid) => {
                if bid.price() >= min_bid {
                    Some(bid)
                } else {
                    None
                }
            }
        }
    }
}

impl Display for AllowedPriceRange {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            format!(
                "AllowedPriceRange(max_ask={}, min_bid={})",
                display_option(&self.max_ask_price),
                display_option(&self.min_bid_price)
            )
            .as_str(),
        )
    }
}

impl Default for AllowedPriceRange {
    fn default() -> Self {
        AllowedPriceRange {
            max_ask_price: None,
            min_bid_price: None,
        }
    }
}
