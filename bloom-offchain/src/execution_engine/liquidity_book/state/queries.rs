use crate::execution_engine::liquidity_book::fragment::Fragment;
use crate::execution_engine::liquidity_book::market_maker::SpotPrice;
use crate::execution_engine::liquidity_book::state::Fragments;
use crate::execution_engine::liquidity_book::types::AbsolutePrice;
use num_rational::Ratio;

pub fn max_by_distance_to_spot<Fr>(fragments: &mut Fragments<Fr>, spot_price: SpotPrice) -> Option<Fr>
where
    Fr: Fragment + Ord + Copy,
{
    let best_bid = fragments.bids.pop_first();
    let best_ask = fragments.asks.pop_first();
    match (best_ask, best_bid) {
        (Some(ask), Some(bid)) => {
            let abs_price = AbsolutePrice::from(spot_price).to_signed();
            let distance_from_ask = abs_price - ask.price().to_signed();
            let distance_from_bid = abs_price - bid.price().to_signed();
            if distance_from_ask > distance_from_bid {
                fragments.insert(bid);
                Some(ask)
            } else if distance_from_ask < distance_from_bid {
                fragments.insert(ask);
                Some(bid)
            } else {
                let choice = _max_by_volume(ask, bid, Some(spot_price));
                if choice == ask {
                    fragments.insert(bid);
                } else {
                    fragments.insert(ask);
                }
                Some(choice)
            }
        }
        (Some(taker), _) | (_, Some(taker)) => Some(taker),
        _ => None,
    }
}

fn _max_by_volume<Fr>(ask: Fr, bid: Fr, spot_price: Option<SpotPrice>) -> Fr
where
    Fr: Fragment,
{
    let index_price = spot_price
        .map(AbsolutePrice::from)
        .unwrap_or_else(|| ask.price() + bid.price() * Ratio::new(1, 2));
    let ask_vol = Ratio::new(ask.input() as u128, 1) * index_price.unwrap();
    let bid_vol = Ratio::new(1, bid.input() as u128) * index_price.unwrap();
    if ask_vol > bid_vol {
        ask
    } else {
        bid
    }
}

pub fn max_by_volume<Fr>(fragments: &mut Fragments<Fr>) -> Option<Fr>
where
    Fr: Fragment + Ord + Copy,
{
    let best_bid = fragments.bids.pop_first();
    let best_ask = fragments.asks.pop_first();
    match (best_ask, best_bid) {
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
    }
}
