use crate::constants::{FEE_DEN, WEIGHT_FEE_DEN};
use crate::data::balance_pool::round_big_number;
use crate::data::order::{Base, Quote};

use bignumber::BigNumber;
use num_rational::Ratio;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};

use log::{info, log};
use std::ops::{Add, Div, Mul};

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
    invariant: u64,
) -> TaggedAmount<Quote> {
    let (base_reserves, base_weight, _quote_reserves, quote_weight, pool_fee) =
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
    // (base_reserves + base_amount * (poolFee / feeDen)) ^ (base_weight / weight_den)
    let base_new_part = (
        // (base_reserves + base_amount * (poolFeeNum - treasuryFeeNum) / feeDen)
        BigNumber::from(base_reserves).add(
            // base_amount * (poolFeeNum - treasuryFeeNum) / feeDen)
            BigNumber::from(base_amount.untag() as f64)
                .mul(BigNumber::from(*pool_fee.numer() as f64).div((BigNumber::from(*pool_fee.denom() as f64)))),
        )
    )
    .pow(&BigNumber::from(base_weight).div(BigNumber::from(WEIGHT_FEE_DEN)));
    println!("base_reserves part: {}", base_reserves.to_string());
    println!("pool fee x: {}", pool_fee_x.to_string());
    println!("pool fee part: {}", pool_fee.to_string());
    println!("base_amount part: {}", base_amount.untag().to_string());
    println!("base_amount part 2: {}", (BigNumber::from(base_amount.untag() as f64)
        .mul(BigNumber::from(*pool_fee.numer() as f64).div((BigNumber::from((*pool_fee.denom() as f64)))))).to_string());
    println!("base part: {}", base_new_part.to_string());
    // (quote_reserves - quote_amount) ^ (quote_weight / weight_den) = invariant / base_new_part
    let quote_new_part = BigNumber::from(invariant as f64).div(base_new_part);
    // quote_amount = quote_reserves - quote_new_part ^ (weight_den / quote_weight)
    println!("quote_new_part {}", quote_new_part.to_string());
    let delta_y = quote_new_part.pow(&BigNumber::from(WEIGHT_FEE_DEN).div(BigNumber::from(quote_weight)));
    let delta_y_rounded = round_big_number(delta_y.clone(), 0);
    println!("delta_y {}", delta_y);
    println!("Test y {}", round_big_number(delta_y.clone(), 0));
    println!(
        "Test y u64 {}",
        round_big_number(delta_y.clone(), 0).as_u64().unwrap()
    );
    let output_amount = _quote_reserves - delta_y_rounded.as_u64().unwrap();
    println!(
        "output_amount {}",
        output_amount
    );
    TaggedAmount::new(output_amount)
}
