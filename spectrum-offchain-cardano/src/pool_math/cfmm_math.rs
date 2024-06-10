use crate::data::order::{Base, Quote};
use crate::data::pool::{Lq, Rx, Ry};

use num_rational::Ratio;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
use std::cmp::min;
use bloom_offchain::execution_engine::liquidity_book::liquidity_bin::Bin;
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use crate::data::pair::order_canonical;

pub fn classic_cfmm_output_amount<X, Y>(
    asset_x: TaggedAssetClass<X>,
    reserves_x: TaggedAmount<X>,
    reserves_y: TaggedAmount<Y>,
    base_asset: TaggedAssetClass<Base>,
    base_amount: TaggedAmount<Base>,
    pool_fee_x: Ratio<u64>,
    pool_fee_y: Ratio<u64>,
) -> TaggedAmount<Quote> {
    let quote_amount = if base_asset.untag() == asset_x.untag() {
        (reserves_y.untag() as u128) * (base_amount.untag() as u128) * (*pool_fee_x.numer() as u128)
            / ((reserves_x.untag() as u128) * (*pool_fee_x.denom() as u128)
            + (base_amount.untag() as u128) * (*pool_fee_x.numer() as u128))
    } else {
        (reserves_x.untag() as u128) * (base_amount.untag() as u128) * (*pool_fee_y.numer() as u128)
            / ((reserves_y.untag() as u128) * (*pool_fee_y.denom() as u128)
            + (base_amount.untag() as u128) * (*pool_fee_y.numer() as u128))
    };
    TaggedAmount::new(quote_amount as u64)
}

pub fn fuse(
    bin: Bin,
    asset_x: TaggedAssetClass<Rx>,
    reserves_x: TaggedAmount<Rx>,
    asset_y: TaggedAssetClass<Ry>,
    reserves_y: TaggedAmount<Ry>,
    treasury_x: TaggedAmount<Rx>,
    treasury_y: TaggedAmount<Ry>,
    treasury_fee: Ratio<u64>,
) -> (TaggedAmount<Rx>, TaggedAmount<Rx>, TaggedAmount<Ry>, TaggedAmount<Ry>) {
    let [base, _] = order_canonical(asset_x.untag(), asset_y.untag());
    let treasury_reduce = (bin.order_input * treasury_fee.numer()) / treasury_fee.denom();
    match bin.amount {
        Side::Bid(quote) => {
            if asset_x.untag() == base {
                (
                    reserves_x - TaggedAmount::new(bin.order_input),
                    treasury_x - TaggedAmount::new(treasury_reduce),
                    reserves_y + TaggedAmount::new(quote),
                    treasury_y
                )
            } else {
                (
                    reserves_x + TaggedAmount::new(quote),
                    treasury_x,
                    reserves_y - TaggedAmount::new(bin.order_input),
                    treasury_y - TaggedAmount::new(treasury_reduce),
                )
            }
        }
        Side::Ask(add_to_base) => {
            if asset_x.untag() == base {
                (
                    reserves_x + TaggedAmount::new(add_to_base),
                    treasury_x,
                    reserves_y - TaggedAmount::new(bin.order_input),
                    treasury_y - TaggedAmount::new(treasury_reduce),
                )
            } else {
                (
                    reserves_x - TaggedAmount::new(bin.order_input),
                    treasury_x - TaggedAmount::new(treasury_reduce),
                    reserves_y + TaggedAmount::new(add_to_base),
                    treasury_y
                )
            }
        }
    }
}

pub fn classic_cfmm_reward_lp(
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

pub fn classic_cfmm_shares_amount(
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
