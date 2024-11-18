use std::ops::{Add, Div, Mul, Sub};

use bignumber::BigNumber;
use log::trace;
use num_bigint::BigInt;
use num_rational::{BigRational, Ratio};
use num_traits::{CheckedAdd, CheckedDiv, CheckedMul, CheckedSub, FromPrimitive, One, Pow, ToPrimitive};
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

pub fn input_estimation_error(
    in_balance: &BigInt,
    out_balance: &BigInt,
    expected_input: &Ratio<BigInt>,
    w_in: &u32,
    w_out: &u32,
    avg_price_num: &BigInt,
    avg_price_denom: &BigInt,
    total_fee_mult_num: &BigInt,
    total_fee_mult_denom: &BigInt,
) -> Option<Ratio<BigInt>> {
    let f_one = 1f64;
    let out_balance_rational = BigRational::from(out_balance.clone());
    let in_balance_rational = BigRational::from(in_balance.clone());
    let total_fee_mult_rational = BigRational::new(total_fee_mult_num.clone(), total_fee_mult_denom.clone());

    let f_pow = Ok::<f64, f64>(*w_out as f64 / *w_in as f64).unwrap_or(f_one);
    let balance_out_root_f64 = Ok::<f64, f64>(out_balance.to_f64()?.powf(f_pow)).unwrap_or(f_one);
    let balance_out_root = BigRational::from_f64(balance_out_root_f64)?;

    let avg_price = BigRational::new(avg_price_num.clone(), avg_price_denom.clone());
    let out_balance_corrected = out_balance_rational.checked_sub(&avg_price.checked_mul(&expected_input)?)?;
    let full_balance_out_root_f64 =
        Ok::<f64, f64>(out_balance_corrected.to_f64()?.powf(f_pow)).unwrap_or(f_one);
    let full_balance_out_root = BigRational::from_f64(full_balance_out_root_f64)?;

    let in_balance_div_fee = in_balance_rational.checked_div(&total_fee_mult_rational)?;
    let estimation_error_num_left = in_balance_div_fee.checked_mul(&balance_out_root)?;
    let estimation_error_num_right =
        full_balance_out_root.checked_mul(&in_balance_div_fee.checked_add(&expected_input)?)?;
    let estimation_error_num = estimation_error_num_left.checked_sub(&estimation_error_num_right)?;

    estimation_error_num.checked_div(&full_balance_out_root)
}

pub fn price_estimation_error(
    in_balance: &BigInt,
    out_balance: &BigInt,
    estimated_input: &BigInt,
    w_in: &u32,
    w_out: &u32,
    avg_price_num: &BigInt,
    avg_price_denom: &BigInt,
) -> Option<Ratio<BigInt>> {
    let f_one = 1f64;
    let f_pow = Ok::<f64, f64>(*w_in as f64 / *w_out as f64).unwrap_or(f_one);
    let balance_in_f64 = in_balance.to_f64()?;
    let balance_out_f64 = out_balance.to_f64()?;
    let balance_in_estimated_f64 = in_balance.checked_add(&estimated_input)?.to_f64()?;
    let balance_in_ratio_root_f64 =
        Ok::<f64, f64>((balance_in_f64 / balance_in_estimated_f64).powf(f_pow)).unwrap_or(f_one);
    let balance_out_corrected_f64 = balance_out_f64 * (f_one - balance_in_ratio_root_f64);

    let balance_out_corrected = BigRational::from_f64(balance_out_corrected_f64)?;

    let estimation_error_num_left = avg_price_num
        .checked_mul(&estimated_input)?
        .checked_mul(balance_out_corrected.denom())?;
    let estimation_error_num_right = balance_out_corrected.numer().checked_mul(&avg_price_denom)?;
    let estimation_error_num = estimation_error_num_left.checked_sub(&estimation_error_num_right)?;
    let estimation_error_denom = estimated_input
        .checked_mul(&avg_price_denom)?
        .checked_mul(balance_out_corrected.denom())?;

    Some(BigRational::new(estimation_error_num, estimation_error_denom))
}

pub fn spot_price_estimation_error(
    in_balance: &BigInt,
    out_balance: &BigInt,
    estimated_input: &BigInt,
    w_in: &u64,
    w_out: &u64,
    avg_price_num: &BigInt,
    avg_price_denom: &BigInt,
    total_fee_num: &BigInt,
    total_fee_denom: &BigInt,
) -> Option<Ratio<BigInt>> {
    let f_one = 1f64;
    let f_pow = Ok::<f64, f64>(*w_in as f64 / *w_out as f64).unwrap_or(f_one);
    let balance_in_f64 = in_balance.to_f64()?;
    let balance_out_f64 = out_balance.to_f64()?;
    let est_in_with_fees = total_fee_num
        .checked_mul(&estimated_input)?
        .checked_div(&total_fee_denom)?;
    let balance_in_estimated_f64 = in_balance.checked_add(&est_in_with_fees)?.to_f64()?;
    let balance_in_ratio_root_f64 =
        Ok::<f64, f64>((balance_in_f64 / balance_in_estimated_f64).powf(f_pow)).unwrap_or(f_one);
    let balance_out_corrected_f64 = balance_out_f64 * (f_one - balance_in_ratio_root_f64);

    let balance_out_corrected = BigRational::from_f64(balance_out_corrected_f64)?;
    let w_in_rational = BigRational::from_u64(*w_in)?;
    let w_out_rational = BigRational::from_u64(*w_out)?;

    let out_balance_rational = BigRational::from(out_balance.clone());
    let est_price_num = out_balance_rational
        .checked_sub(&balance_out_corrected)?
        .checked_mul(&w_in_rational)?;
    let est_price_denom =
        BigRational::from(in_balance.checked_add(&estimated_input)?).checked_mul(&w_out_rational)?;
    let est_price = est_price_num.checked_div(&est_price_denom)?;

    let est_err = BigRational::new(avg_price_num.clone(), avg_price_denom.clone()).checked_sub(&est_price)?;
    Some(est_err)
}

pub fn simple_estimate_inp_by_spot_price(
    in_balance: &BigInt,
    out_balance: &BigInt,
    w_in: &u64,
    w_out: &u64,
    final_spot_price_num: &BigInt,
    final_spot_price_denom: &BigInt,
) -> Option<BigInt> {
    let f_one = f64::one();
    let f_pow = Ok::<f64, f64>(*w_in as f64 / *w_out as f64).unwrap_or(f_one);

    let balance_in_f64 = in_balance.to_f64()?;
    let balance_in_root_f64 = Ok::<f64, f64>(balance_in_f64.powf(f_pow)).unwrap_or(f_one);
    let balance_in_root = BigRational::from_f64(balance_in_root_f64)?;

    let f_pow_s = Ok::<f64, f64>(*w_out as f64 / (*w_out as f64 + *w_in as f64)).unwrap_or(f_one);
    let final_spot_price_f64 =
        final_spot_price_num.to_f64().unwrap() / final_spot_price_denom.to_f64().unwrap();
    let final_spot_price = BigRational::from_f64(final_spot_price_f64)?;
    let w_in_rational = BigRational::from_u64(*w_in)?;
    let w_out_rational = BigRational::from_u64(*w_out)?;

    let balance_in_rational = BigRational::from(in_balance.clone());
    let balance_out_rational = BigRational::from(out_balance.clone());

    let s_num = balance_out_rational
        .checked_mul(&w_in_rational)?
        .checked_mul(&balance_in_root)?;
    let s_denom = final_spot_price.checked_mul(&w_out_rational)?;

    let s = s_num.checked_div(&s_denom)?;
    let s_pow = Ok::<f64, f64>(s.to_f64()?.powf(f_pow_s)).unwrap_or(f_one);
    let est_balance_in = BigRational::from_f64(s_pow)?;

    let est_in = est_balance_in.checked_sub(&balance_in_rational)?;
    Some(est_in.to_integer())
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
    let pool_fee_num_u512 = U512::from(*pool_fee.numer());
    let pool_fee_denom_u512 = U512::from(*pool_fee.denom());

    let additional_part = U512::from(base_amount)
        .mul(pool_fee_num_u512)
        .mul(pool_fee_denom_u512);

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
    use std::time::SystemTime;

    use num_bigint::BigInt;
    use num_rational::Ratio;
    use num_traits::ToPrimitive;
    use primitive_types::U512;

    use spectrum_cardano_lib::AssetClass::Native;
    use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};

    use crate::pool_math::balance_math::{
        balance_cfmm_output_amount, balance_cfmm_output_amount_old, calculate_new_invariant_bn_u,
        price_estimation_error, simple_estimate_inp_by_spot_price, spot_price_estimation_error,
    };

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
        assert_eq!(r_new, TaggedAmount::new(2616672813));
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
    #[test]
    fn price_estimation_error_test() {
        let r_in = BigInt::from(1_000_000_000);
        let r_out = BigInt::from(1_000_000_000);
        let r_inp = BigInt::from(2);
        let w_in = 4;
        let w_out = 1;
        let p_num = BigInt::from(39999);
        let p_denom = BigInt::from(10000);

        let est = price_estimation_error(&r_in, &r_out, &r_inp, &w_in, &w_out, &p_num, &p_denom);
        // assert_eq!(est.clone().unwrap().denom().to_f64().unwrap(), 0f64);
        assert_eq!(est.unwrap().to_f64().unwrap(), -9.98868722741463e-5)
    }
    #[test]
    fn spot_price_estimation_error_test() {
        let r_in = BigInt::from(2000000);
        let r_out = BigInt::from(2000000);
        let r_inp = BigInt::from(78_392);
        let w_in = 4;
        let w_out = 1;
        let p_num = BigInt::from(3843081685656419u64);
        let p_denom = BigInt::from(1125899906842624u64);
        let f_num = BigInt::from(778);
        let f_denom = BigInt::from(1000);

        let est = spot_price_estimation_error(
            &r_in, &r_out, &r_inp, &w_in, &w_out, &p_num, &p_denom, &f_num, &f_denom,
        );
        assert_eq!(est.unwrap().to_f64().unwrap(), -6.465684169603373e-6)
    }

    #[test]
    fn simple_estimate_inp_by_spot_price_test() {
        let r_in = BigInt::from(2000);
        let r_out = BigInt::from(1000);
        let w_in = 3;
        let w_out = 2;
        let p_num = BigInt::from(21);
        let p_denom = BigInt::from(10);

        let est = simple_estimate_inp_by_spot_price(&r_in, &r_out, &w_in, &w_out, &p_num, &p_denom);
        assert_eq!(est.unwrap().to_i64().unwrap(), -675)
    }
}
