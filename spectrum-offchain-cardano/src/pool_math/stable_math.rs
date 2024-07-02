use crate::data::pool::{Lq, Rx, Ry};
use bignumber::BigNumber;
use spectrum_cardano_lib::TaggedAmount;
use std::cmp::min;
use std::ops::{Div, Mul};

pub fn stable_cfmm_reward_lp(
    reserves_x: TaggedAmount<Rx>,
    reserves_y: TaggedAmount<Ry>,
    liquidity: TaggedAmount<Lq>,
    in_x_amount: u64,
    in_y_amount: u64,
) -> Option<(TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>)> {
    let min_by_x =
        ((in_x_amount as u128) * (liquidity.untag() as u128)).checked_div(reserves_x.untag() as u128)? as u64;
    let min_by_y =
        ((in_y_amount as u128) * (liquidity.untag() as u128)).checked_div(reserves_y.untag() as u128)? as u64;
    let (change_by_x, change_by_y): (u64, u64) = if min_by_x == min_by_y {
        (0, 0)
    } else {
        if min_by_x < min_by_y {
            let y_to_deposit: u64 = BigNumber::from(min_by_x as f64)
                .div(BigNumber::from(liquidity.untag() as f64))
                .mul(BigNumber::from(reserves_y.untag() as f64))
                .value
                .to_int()
                .value()
                .try_into()
                .unwrap();
            (0, in_y_amount - y_to_deposit)
        } else {
            let x_to_deposit: u64 = BigNumber::from(min_by_y as f64)
                .div(BigNumber::from(liquidity.untag() as f64))
                .mul(BigNumber::from(reserves_x.untag() as f64))
                .value
                .to_int()
                .value()
                .try_into()
                .unwrap();
            (0, in_x_amount - x_to_deposit)
        }
    };
    let unlocked_lq: u64 = min(min_by_x, min_by_y);

    Some((
        TaggedAmount::new(unlocked_lq),
        TaggedAmount::new(change_by_x),
        TaggedAmount::new(change_by_y),
    ))
}
