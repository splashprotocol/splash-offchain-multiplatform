use crate::data::order::{Base, Quote};
use crate::data::pool::{Lq, Rx, Ry};

use num_rational::Ratio;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
use std::cmp::min;

pub const UNTOUCHABLE_LOVELACE_AMOUNT: u64 = 3_000_000;

pub fn classic_cfmm_output_amount<X, Y>(
    asset_x: TaggedAssetClass<X>,
    asset_y: TaggedAssetClass<Y>,
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
    let (output_asset, liquidity) = if base_asset.untag() == asset_x.untag() {
        (asset_y.untag(), reserves_y.untag())
    } else {
        (asset_x.untag(), reserves_x.untag())
    };
    let capped_quote = if output_asset.is_native() {
        let amount_left = liquidity - quote_amount as u64;
        if amount_left < UNTOUCHABLE_LOVELACE_AMOUNT {
            liquidity - UNTOUCHABLE_LOVELACE_AMOUNT
        } else {
            quote_amount as u64
        }
    } else {
        quote_amount as u64
    };
    TaggedAmount::new(capped_quote)
}

pub fn classic_cfmm_reward_lp(
    reserves_x: TaggedAmount<Rx>,
    reserves_y: TaggedAmount<Ry>,
    liquidity: TaggedAmount<Lq>,
    in_x_amount: u64,
    in_y_amount: u64,
) -> Option<(TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>)> {
    let min_by_x =
        ((in_x_amount as u128) * (liquidity.untag() as u128)).checked_div(reserves_x.untag() as u128)?;
    let min_by_y =
        ((in_y_amount as u128) * (liquidity.untag() as u128)).checked_div(reserves_y.untag() as u128)?;
    let (change_by_x, change_by_y): (u64, u64) = if min_by_x == min_by_y {
        (0, 0)
    } else {
        if min_by_x < min_by_y {
            (
                0,
                (((min_by_y - min_by_x) * (reserves_y.untag() as u128))
                    .checked_div(liquidity.untag() as u128))? as u64,
            )
        } else {
            (
                (((min_by_x - min_by_y) * (reserves_x.untag() as u128))
                    .checked_div(liquidity.untag() as u128))? as u64,
                0,
            )
        }
    };
    let unlocked_lq = min(min_by_x, min_by_y) as u64;
    Some((
        TaggedAmount::new(unlocked_lq),
        TaggedAmount::new(change_by_x),
        TaggedAmount::new(change_by_y),
    ))
}

pub fn classic_cfmm_shares_amount(
    reserves_x: TaggedAmount<Rx>,
    reserves_y: TaggedAmount<Ry>,
    liquidity: TaggedAmount<Lq>,
    burned_lq: TaggedAmount<Lq>,
) -> Option<(TaggedAmount<Rx>, TaggedAmount<Ry>)> {
    let x_amount =
        ((burned_lq.untag() as u128) * (reserves_x.untag() as u128)).checked_div(liquidity.untag() as u128)?;
    let y_amount =
        ((burned_lq.untag() as u128) * (reserves_y.untag() as u128)).checked_div(liquidity.untag() as u128)?;

    Some((
        TaggedAmount::new(x_amount as u64),
        TaggedAmount::new(y_amount as u64),
    ))
}
