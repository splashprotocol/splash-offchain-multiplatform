use std::ops::{Add, Div, Mul, Sub};

use bignumber::BigNumber;
use num_rational::Ratio;
use primitive_types::U512;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};

use crate::data::order::{Base, Quote};

pub fn balance_cfmm_output_amount<X, Y>(
    asset_x: TaggedAssetClass<X>,
    reserves_x: TaggedAmount<X>,
    x_weight: u64,
    reserves_y: TaggedAmount<Y>,
    y_weight: u64,
    base_asset: TaggedAssetClass<Base>,
    base_amount: TaggedAmount<Base>,
    pool_fee_x: Ratio<u64>,
    pool_fee_y: Ratio<u64>,
    invariant: U512,
) -> TaggedAmount<Quote> {
    let (base_reserves, base_weight, quote_reserves, quote_weight, pool_fee) =
        if asset_x.untag() == base_asset.untag() {
            (
                reserves_x.untag() as f64,
                x_weight as f64,
                reserves_y.untag(),
                y_weight as f64,
                pool_fee_x,
            )
        } else {
            (
                reserves_y.untag() as f64,
                y_weight as f64,
                reserves_x.untag(),
                x_weight as f64,
                pool_fee_y,
            )
        };
    let base_new_part =
        calculate_base_part_with_fee(base_reserves, base_weight, base_amount.untag() as f64, pool_fee);
    // (quote_reserves - quote_amount) ^ quote_weight = invariant / base_new_part
    let quote_new_part = BigNumber::from(invariant).div(base_new_part.clone());
    let delta_y = quote_new_part
        .clone()
        .pow(&BigNumber::from(1).div(BigNumber::from(quote_weight)));
    let delta_y_rounded = <u64>::try_from(delta_y.value.to_int().value()).unwrap();
    // quote_amount = quote_reserves - quote_new_part ^ (1 / quote_weight)
    let mut pre_output_amount = quote_reserves - delta_y_rounded;
    // we should find the most approximate value to previous invariant
    while calculate_new_invariant_bn(
        base_reserves,
        base_weight,
        base_amount.untag() as f64,
        quote_reserves as f64,
        quote_weight,
        pre_output_amount as f64,
        pool_fee,
    ) <= invariant
    {
        pre_output_amount -= 1
    }
    TaggedAmount::new(pre_output_amount)
}
fn calculate_new_invariant_bn(
    base_reserves: f64,
    base_weight: f64,
    base_amount: f64,
    quote_reserves: f64,
    quote_weight: f64,
    quote_output: f64,
    pool_fee: Ratio<u64>,
) -> U512 {
    let additional_part = ((base_amount as u64) * *pool_fee.numer()) / pool_fee.denom();

    let base_new_part = BigNumber::from(base_reserves)
        .add(BigNumber::from(additional_part as f64))
        .pow(&BigNumber::from(base_weight));

    let quote_part = BigNumber::from(quote_reserves)
        .sub(BigNumber::from(quote_output))
        .pow(&BigNumber::from(quote_weight));

    U512::from_str_radix(base_new_part.mul(quote_part).to_string().as_str(), 10).unwrap()
}

// (base_reserves + base_amount * (poolFee / feeDen)) ^ base_weight
fn calculate_base_part_with_fee(
    base_reserves: f64,
    base_weight: f64,
    base_amount: f64,
    pool_fee: Ratio<u64>,
) -> BigNumber {
    BigNumber::from(base_reserves)
        .add(
            // base_amount * (poolFeeNum - treasuryFeeNum) / feeDen)
            BigNumber::from(base_amount).mul(
                BigNumber::from(*pool_fee.numer() as f64).div((BigNumber::from(*pool_fee.denom() as f64))),
            ),
        )
        .pow(&BigNumber::from(base_weight))
}
