use std::cmp::min;

use num_bigint::BigInt;
use num_rational::Ratio;
use num_traits::ToPrimitive;

use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};

use crate::data::order::{Base, Quote};
use crate::data::pool::{Lq, Rx, Ry};

pub fn classic_cfmm_output_amount<X, Y>(
    asset_x: TaggedAssetClass<X>,
    reserves_x: TaggedAmount<X>,
    reserves_y: TaggedAmount<Y>,
    base_asset: TaggedAssetClass<Base>,
    base_amount: TaggedAmount<Base>,
    pool_fee_x: Ratio<u64>,
    pool_fee_y: Ratio<u64>,
) -> Option<TaggedAmount<Quote>> {
    // Pool params according to the side:
    let (in_balance, out_balance, total_fee_mul_num, total_fee_mul_denom) =
        if base_asset.untag() == asset_x.untag() {
            (
                BigInt::from(reserves_x.untag()),
                BigInt::from(reserves_y.untag()),
                BigInt::from(*pool_fee_x.numer()),
                BigInt::from(*pool_fee_x.denom()),
            )
        } else {
            (
                BigInt::from(reserves_y.untag()),
                BigInt::from(reserves_x.untag()),
                BigInt::from(*pool_fee_y.numer()),
                BigInt::from(*pool_fee_y.denom()),
            )
        };
    let base_amount_in = BigInt::from(base_amount.untag());

    let quote_amount_num = out_balance
        .checked_mul(&base_amount_in)?
        .checked_mul(&total_fee_mul_num)?;
    let quote_amount_denom_left = in_balance.checked_mul(&total_fee_mul_denom)?;
    let quote_amount_denom_right = base_amount_in.checked_mul(&total_fee_mul_num)?;
    let quote_amount_denom = quote_amount_denom_left.checked_add(&quote_amount_denom_right)?;
    let quote_amount = quote_amount_num.checked_div(&quote_amount_denom)?.to_u64()?;
    Some(TaggedAmount::new(quote_amount))
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
