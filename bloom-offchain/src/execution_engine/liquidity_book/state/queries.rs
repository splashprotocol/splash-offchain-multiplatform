use crate::execution_engine::liquidity_book::market_maker::SpotPrice;
use crate::execution_engine::liquidity_book::market_taker::MarketTaker;
use crate::execution_engine::liquidity_book::state::{AllowedPriceRange, MarketTakers};
use crate::execution_engine::liquidity_book::types::AbsolutePrice;
use num_rational::Ratio;

pub fn max_by_distance_to_spot<Fr>(
    fragments: &mut MarketTakers<Fr>,
    spot_price: SpotPrice,
    range: AllowedPriceRange,
) -> Option<Fr>
where
    Fr: MarketTaker + Ord + Copy,
{
    let best_bid = fragments.bids.first().and_then(|tk| range.test_bid(*tk));
    let best_ask = fragments.asks.first().and_then(|tk| range.test_ask(*tk));
    let choice = match (best_ask, best_bid) {
        (Some(ask), Some(bid)) => {
            let abs_price = AbsolutePrice::from(spot_price).to_signed();
            let distance_from_ask = abs_price - ask.price().to_signed();
            let distance_from_bid = (abs_price - bid.price().to_signed()) * -1;
            if distance_from_ask > distance_from_bid {
                Some(ask)
            } else if distance_from_ask < distance_from_bid {
                Some(bid)
            } else {
                Some(_max_by_volume(ask, bid, Some(spot_price)))
            }
        }
        (Some(taker), _) | (_, Some(taker)) => Some(taker),
        _ => None,
    };
    if let Some(taker) = &choice {
        fragments.remove(taker);
    }
    choice
}

fn _max_by_volume<Fr>(ask: Fr, bid: Fr, spot_price: Option<SpotPrice>) -> Fr
where
    Fr: MarketTaker,
{
    let raw_spot_price = spot_price.map(|sp| sp.unwrap());
    let ask_vol = Ratio::new(ask.input() as u128, 1);
    let bid_vol = Ratio::new(1, bid.input() as u128) * raw_spot_price.unwrap_or_else(|| bid.price().unwrap());
    if ask_vol > bid_vol {
        ask
    } else {
        bid
    }
}

pub fn max_by_volume<Fr>(fragments: &mut MarketTakers<Fr>, range: AllowedPriceRange) -> Option<Fr>
where
    Fr: MarketTaker + Ord + Copy,
{
    let best_bid = fragments.bids.first().and_then(|tk| range.test_bid(*tk));
    let best_ask = fragments.asks.first().and_then(|tk| range.test_ask(*tk));
    let choice = match (best_ask, best_bid) {
        (Some(ask), Some(bid)) => {
            let choice = _max_by_volume(ask, bid, None);
            if choice == ask {
                fragments.insert(bid);
            } else {
                fragments.insert(ask);
            }
            Some(choice)
        }
        (Some(taker), _) | (_, Some(taker)) => Some(taker),
        _ => None,
    };
    if let Some(taker) = &choice {
        fragments.remove(taker);
    }
    choice
}

#[cfg(test)]
mod tests {
    use crate::execution_engine::liquidity_book::market_maker::SpotPrice;
    use crate::execution_engine::liquidity_book::side::Side;
    use crate::execution_engine::liquidity_book::state::price_range::AllowedPriceRange;
    use crate::execution_engine::liquidity_book::state::queries::max_by_distance_to_spot;
    use crate::execution_engine::liquidity_book::state::tests::SimpleOrderPF;
    use crate::execution_engine::liquidity_book::state::MarketTakers;
    use crate::execution_engine::liquidity_book::types::AbsolutePrice;

    #[test]
    fn select_by_max_distance() {
        let mut mt: MarketTakers<SimpleOrderPF> = MarketTakers::new();
        let ask = SimpleOrderPF::new(
            Side::Ask,
            1000000,
            AbsolutePrice::new_unsafe(13964959385539833, 5000000000000000),
            1100000,
            900000,
        );
        let bid = SimpleOrderPF::new(
            Side::Bid,
            6666700000,
            AbsolutePrice::new_unsafe(250000, 127183),
            1100000,
            900000,
        );
        let spot = SpotPrice::from(AbsolutePrice::new_unsafe(35953787, 11755056));
        mt.asks.insert(ask);
        mt.bids.insert(bid);
        let choice = max_by_distance_to_spot(&mut mt, spot, AllowedPriceRange::default());
        assert_eq!(choice.unwrap().side, Side::Ask);
    }
}
