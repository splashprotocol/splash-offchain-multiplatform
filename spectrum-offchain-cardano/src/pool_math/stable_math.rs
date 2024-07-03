use std::cmp::min;
use std::ops::Div;
use bignumber::BigNumber;
use log::info;
use spectrum_cardano_lib::TaggedAmount;
use crate::data::pool::{Lq, Rx, Ry};

pub fn stable_cfmm_reward_lp(
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
                BigNumber::from(((((min_by_y - min_by_x) * (reserves_y.untag() as u128)) as f64)
                    .div(liquidity.untag() as f64))).to_precision(1).value.to_f64().value() as u64,
            )
        } else {
            (
                BigNumber::from(((((min_by_x - min_by_y) * (reserves_x.untag() as u128)) as f64)
                    .div(liquidity.untag() as f64))).to_precision(1).value.to_f64().value() as u64,
                0,
            )
        }
    };
    let unlocked_lq: u64 = min(min_by_x, min_by_y) as u64;
    Some((
        TaggedAmount::new(unlocked_lq),
        TaggedAmount::new(change_by_x),
        TaggedAmount::new(change_by_y),
    ))
}