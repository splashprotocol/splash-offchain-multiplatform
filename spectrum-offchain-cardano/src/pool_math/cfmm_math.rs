use crate::data::order::{Base, Quote};
use crate::data::pool::{Lq, Rx, Ry};
use num_rational::Ratio;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
use std::cmp::min;

pub fn output_amount(
    asset_x: TaggedAssetClass<Rx>,
    reserves_x: TaggedAmount<Rx>,
    reserves_y: TaggedAmount<Ry>,
    base_asset: TaggedAssetClass<Base>,
    base_amount: TaggedAmount<Base>,
    lp_fee_x: Ratio<u64>,
    lp_fee_y: Ratio<u64>,
) -> TaggedAmount<Quote> {
    let quote_amount = if base_asset.untag() == asset_x.untag() {
        (reserves_y.untag() as u128) * (base_amount.untag() as u128) * (*lp_fee_x.numer() as u128)
            / ((reserves_x.untag() as u128) * (*lp_fee_x.denom() as u128)
                + (base_amount.untag() as u128) * (*lp_fee_x.numer() as u128))
    } else {
        (reserves_x.untag() as u128) * (base_amount.untag() as u128) * (*lp_fee_y.numer() as u128)
            / ((reserves_y.untag() as u128) * (*lp_fee_y.denom() as u128)
                + (base_amount.untag() as u128) * (*lp_fee_y.numer() as u128))
    };
    TaggedAmount::new(quote_amount as u64)
}

pub fn treasury_amount(
    asset_x: TaggedAssetClass<Rx>,
    reserves_x: TaggedAmount<Rx>,
    reserves_y: TaggedAmount<Ry>,
    base_asset: TaggedAssetClass<Base>,
    base_amount: TaggedAmount<Base>,
    lp_fee_x: Ratio<u64>,
    treasury_fee_x: Ratio<u64>,
    lp_fee_y: Ratio<u64>,
    treasury_fee_y: Ratio<u64>,
) -> (TaggedAmount<Rx>, TaggedAmount<Ry>) {
    if base_asset.untag() == asset_x.untag() {
        let y_amount = (reserves_x.untag() as u128) * (reserves_y.untag() as u128)
            / ((reserves_x.untag() as u128) + (base_amount.untag() as u128));
        //let lp_amount = y_amount * ((1 - lp_fee_y) as u128) * treasury_fee_x;
        (TaggedAmount::new(0), TaggedAmount::new(0))
    } else {
        let y_amount = (reserves_x.untag() as u128) * (reserves_y.untag() as u128)
            / ((reserves_x.untag() as u128) + (base_amount.untag() as u128));
        //let lp_amount = y_amount * ((1 - lp_fee_y) as u128) * treasury_fee_x;
        (TaggedAmount::new(0), TaggedAmount::new(0))
    }
}

pub fn reward_lp(
    reserves_x: TaggedAmount<Rx>,
    reserves_y: TaggedAmount<Ry>,
    liquidity: TaggedAmount<Lq>,
    in_x_amount: u64,
    in_y_amount: u64,
) -> (TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>) {
    let min_by_x = (in_x_amount as u128) * (liquidity.untag() as u128) / (reserves_x.untag() as u128);
    let min_by_y = (in_y_amount as u128) * (liquidity.untag() as u128) / (reserves_y.untag() as u128);
    let (change_by_x, change_by_y): (u64, u64) = if min_by_x == min_by_y {
        (0, 0)
    } else {
        if min_by_x < min_by_y {
            (
                0,
                ((min_by_y - min_by_x) * (reserves_y.untag() as u128) / (liquidity.untag() as u128)) as u64,
            )
        } else {
            (
                ((min_by_x - min_by_y) * (reserves_x.untag() as u128) / (liquidity.untag() as u128)) as u64,
                0,
            )
        }
    };
    let unlocked_lq = min(min_by_x, min_by_y) as u64;
    (
        TaggedAmount::new(unlocked_lq),
        TaggedAmount::new(change_by_x),
        TaggedAmount::new(change_by_y),
    )
}

pub fn shares_amount(
    reserves_x: TaggedAmount<Rx>,
    reserves_y: TaggedAmount<Ry>,
    liquidity: TaggedAmount<Lq>,
    burned_lq: TaggedAmount<Lq>,
) -> (TaggedAmount<Rx>, TaggedAmount<Ry>) {
    let x_amount = (burned_lq.untag() as u128) * (reserves_x.untag() as u128) / (liquidity.untag() as u128);
    let y_amount = (burned_lq.untag() as u128) * (reserves_y.untag() as u128) / (liquidity.untag() as u128);

    (
        TaggedAmount::new(x_amount as u64),
        TaggedAmount::new(y_amount as u64),
    )
}
