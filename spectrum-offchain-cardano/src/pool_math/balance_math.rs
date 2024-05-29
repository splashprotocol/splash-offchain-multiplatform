use std::ops::{Add, Div, Mul, Sub};

use bignumber::BigNumber;
use log::trace;
use num_rational::Ratio;
use primitive_types::U512;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};

use crate::data::order::{Base, Quote};

pub fn balance_cfmm_output_amount_old<X, Y>(
    asset_x: TaggedAssetClass<X>,
    reserves_x: TaggedAmount<X>,
    x_weight: u64,
    reserves_y: TaggedAmount<Y>,
    y_weight: u64,
    base_asset: TaggedAssetClass<Base>,
    base_amount: TaggedAmount<Base>,
    pool_fee_x: Ratio<u64>,
    pool_fee_y: Ratio<u64>,
) -> TaggedAmount<Quote> {
    trace!(
        "balance_cfmm_output_amount({:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?})",
        asset_x,
        reserves_x,
        x_weight,
        reserves_y,
        y_weight,
        base_asset,
        base_amount,
        pool_fee_x,
        pool_fee_y,
    );
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
    let invariant = U512::from(base_reserves as u64)
        .pow(U512::from(base_weight as u64))
        .mul(U512::from(quote_reserves).pow(U512::from(quote_weight as u64)));
    let base_new_part =
        calculate_base_part_with_fee_old(base_reserves, base_weight, base_amount.untag() as f64, pool_fee);
    // (quote_reserves - quote_amount) ^ quote_weight = invariant / base_new_part
    let quote_new_part = BigNumber::from(invariant).div(base_new_part.clone());
    let delta_y = quote_new_part
        .clone()
        .pow(&BigNumber::from(1).div(BigNumber::from(quote_weight)));
    let delta_y_rounded = <u64>::try_from(delta_y.value.to_int().value()).unwrap();
    // quote_amount = quote_reserves - quote_new_part ^ (1 / quote_weight)
    let mut pre_output_amount = quote_reserves - delta_y_rounded;
    // we should find the most approximate value to previous invariant
    let mut num_loops = 0;
    trace!(
        "balance_cfmm_output_amount::calculate_new_invariant_bn({:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?})",
        base_reserves,
        base_weight,
        base_amount.untag() as f64,
        quote_reserves as f64,
        quote_weight,
        pre_output_amount as f64,
        pool_fee,
    );
    while calculate_new_invariant_bn_u(
        base_reserves as u64,
        base_weight as u64,
        base_amount.untag(),
        quote_reserves,
        quote_weight as u64,
        pre_output_amount,
        pool_fee,
    ) < invariant
    {
        num_loops += 1;
        pre_output_amount -= 1
    }
    TaggedAmount::new(pre_output_amount)
}

fn calculate_base_part_with_fee_old(
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
) -> TaggedAmount<Quote> {
    trace!(
        "balance_cfmm_output_amount({:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?})",
        asset_x,
        reserves_x,
        x_weight,
        reserves_y,
        y_weight,
        base_asset,
        base_amount,
        pool_fee_x,
        pool_fee_y
    );
    let (base_reserves, base_weight, quote_reserves, quote_weight, pool_fee) =
        if asset_x.untag() == base_asset.untag() {
            (
                reserves_x.untag(),
                x_weight,
                reserves_y.untag(),
                y_weight,
                pool_fee_x,
            )
        } else {
            (
                reserves_y.untag(),
                y_weight,
                reserves_x.untag(),
                x_weight,
                pool_fee_y,
            )
        };
    let invariant = U512::from(base_reserves)
        .pow(U512::from(base_weight))
        .mul(U512::from(quote_reserves).pow(U512::from(quote_weight)));
    let base_new_part =
        calculate_base_part_with_fee(base_reserves, base_weight, base_amount.untag(), pool_fee);
    // (quote_reserves - quote_amount) ^ quote_weight = invariant / base_new_part
    let quote_new_part = BigNumber::from(invariant).div(BigNumber::from(base_new_part));
    let delta_y = quote_new_part.pow(&BigNumber::from(1).div(BigNumber::from(quote_weight as f64)));
    let delta_y_rounded = <u64>::try_from(delta_y.value.to_int().value()).unwrap();
    // quote_amount = quote_reserves - quote_new_part ^ (1 / quote_weight)
    let mut pre_output_amount = quote_reserves - delta_y_rounded;
    // we should find the most approximate value to previous invariant
    let mut num_loops = 0;
    trace!(
        "balance_cfmm_output_amount::calculate_new_invariant_bn({:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?})",
        base_reserves,
        base_weight,
        base_amount.untag() as f64,
        quote_reserves as f64,
        quote_weight,
        pre_output_amount as f64,
        pool_fee,
    );
    while calculate_new_invariant_bn_u(
        base_reserves,
        base_weight,
        base_amount.untag(),
        quote_reserves,
        quote_weight,
        pre_output_amount,
        pool_fee,
    ) < invariant
    {
        num_loops += 1;
        pre_output_amount -= 1
    }
    trace!(
        "balance_cfmm_output_amount loops done: {}, final pre_output_amount: {}",
        num_loops,
        pre_output_amount
    );
    TaggedAmount::new(pre_output_amount)
}

fn calculate_new_invariant_bn_u(
    base_reserves: u64,
    base_weight: u64,
    base_amount: u64,
    quote_reserves: u64,
    quote_weight: u64,
    quote_output: u64,
    pool_fee: Ratio<u64>,
) -> U512 {
    let additional_part = base_amount * *pool_fee.numer() / pool_fee.denom();

    let base_new_part = U512::from(base_reserves)
        .add(U512::from(additional_part))
        .pow(U512::from(base_weight));

    let quote_part = U512::from(quote_reserves)
        .sub(U512::from(quote_output))
        .pow(U512::from(quote_weight));

    base_new_part.mul(quote_part)
}

// (base_reserves + base_amount * (poolFee / feeDen)) ^ base_weight
fn calculate_base_part_with_fee(
    base_reserves: u64,
    base_weight: u64,
    base_amount: u64,
    pool_fee: Ratio<u64>,
) -> U512 {
    U512::from(base_reserves)
        .add(
            U512::from(base_amount)
                .mul(U512::from(*pool_fee.numer()))
                .div(U512::from(*pool_fee.denom())),
        )
        .pow(U512::from(base_weight))
}

#[cfg(test)]
mod tests {
    use crate::pool_math::balance_math::{
        balance_cfmm_output_amount, balance_cfmm_output_amount_old, calculate_new_invariant_bn_u,
    };
    use num_rational::Ratio;
    use primitive_types::U512;
    use spectrum_cardano_lib::AssetClass::Native;
    use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
    use std::time::SystemTime;

    #[test]
    fn bench_calculate_new_invariant_bn() {
        let a = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let r_new = balance_cfmm_output_amount::<u32, u64>(
            TaggedAssetClass::new(Native),
            TaggedAmount::new(138380931),
            1,
            TaggedAmount::new(941773308860),
            4,
            TaggedAssetClass::new(Native),
            TaggedAmount::new(1553810),
            Ratio::new(9967, 10000),
            Ratio::new(9967, 10000),
        );
        let r_old = balance_cfmm_output_amount_old::<u32, u64>(
            TaggedAssetClass::new(Native),
            TaggedAmount::new(138380931),
            1,
            TaggedAmount::new(941773308860),
            4,
            TaggedAssetClass::new(Native),
            TaggedAmount::new(1553810),
            Ratio::new(9967, 10000),
            Ratio::new(9967, 10000),
        );
        let b = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        println!(
            "{} millis elapsed, final r: {:?}, legacy variant: {:?}",
            b - a,
            r_new,
            r_old
        );
        assert_eq!(r_new, TaggedAmount::new(2631943370));
    }

    #[test]
    fn bench_calculate_new_invariant_bn_u() {
        let a = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let mut qo = 926163164560u64;
        let mut loops_done = 0;
        let mut r = U512::from(0);
        while loops_done < 684 {
            r = calculate_new_invariant_bn_u(
                147947582u64,
                1u64,
                1553810u64,
                926163164561u64,
                4u64,
                qo,
                Ratio::new(9967, 10000),
            );
            qo -= 1u64;
            loops_done += 1;
        }
        let b = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        println!("{} millis elapsed, final qo: {}, r: {}", b - a, qo, r);
    }
}
