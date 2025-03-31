use std::ops::Add;

use bignumber::BigNumber;
use dashu_base::UnsignedAbs;
use primitive_types::U512;

use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};

use crate::data::order::{Base, Quote};
use crate::data::stable_pool_t2t::StablePoolT2T;

const MAX_SWAP_ERROR: u64 = 2;
const N_TRADABLE_ASSETS: usize = 2;

pub fn calc_stable_swap<X, Y>(
    asset_x: TaggedAssetClass<X>,
    reserves_x: TaggedAmount<X>,
    x_mult: u64,
    reserves_y: TaggedAmount<Y>,
    y_mult: u64,
    base_asset: TaggedAssetClass<Base>,
    base_amount: TaggedAmount<Base>,
    an2n: u64,
) -> Option<TaggedAmount<Quote>> {
    if an2n == 0 && x_mult == 0 && y_mult == 0 {
        return None;
    }
    let an2n_calc = U512::from(an2n);
    let (base_reserves, base_mult, quote_reserves, quote_mult) = if asset_x.untag() == base_asset.untag() {
        (reserves_x.untag(), x_mult, reserves_y.untag(), y_mult)
    } else {
        (reserves_y.untag(), y_mult, reserves_x.untag(), x_mult)
    };

    let quote_calc = U512::from(quote_reserves * quote_mult);
    let base_initial = U512::from(base_reserves * base_mult.clone());
    let base_calc = base_initial + U512::from(base_amount.untag() * base_mult);

    let s = base_calc;
    let p = base_calc;

    let nn = U512::from(N_TRADABLE_ASSETS.pow(N_TRADABLE_ASSETS as u32));
    let ann = an2n_calc / nn;

    let d = calculate_invariant(&base_initial, &quote_calc, &an2n_calc)?;
    let b = s + d / ann;
    let dn1 = vec![d; N_TRADABLE_ASSETS + 1]
        .into_iter()
        .fold(U512::one(), |a, b| a * b);
    let c = dn1 / nn / p / ann;

    let unit = U512::from(1);

    let asset_to_initial = quote_calc.clone();
    let mut asset_to = quote_calc.clone();
    let mut asset_to_previous: U512 = U512::from(0);
    let mut abs_err = unit;
    while abs_err >= unit {
        asset_to_previous = asset_to;
        asset_to = (asset_to_previous * asset_to_previous + c) / (U512::from(2) * asset_to_previous + b - d);
        abs_err = if asset_to > asset_to_previous {
            asset_to - asset_to_previous
        } else {
            asset_to_previous - asset_to
        };
    }

    let d_new = calculate_invariant(&base_calc, &asset_to, &an2n_calc)?;
    let d_after = if d_new > d { d_new } else { d };

    let mut valid_inv = check_exact_invariant(
        &U512::from(quote_mult),
        &base_initial,
        &asset_to_initial,
        &base_calc,
        &asset_to,
        &d_after,
        &nn,
        &an2n_calc,
    )?;
    let mut counter = 0;

    if !valid_inv {
        while !valid_inv && counter < 255 {
            asset_to += unit;
            valid_inv = check_exact_invariant(
                &U512::from(quote_mult),
                &base_initial,
                &asset_to_initial,
                &base_calc,
                &asset_to,
                &d_after,
                &nn,
                &an2n_calc,
            )?;
            counter += 1;
        }
    }

    let quote_amount_pure_delta = asset_to_initial - asset_to;
    let output = (quote_amount_pure_delta / quote_mult).as_u64();
    Some(TaggedAmount::new(output))
}

pub fn calculate_invariant_error_sgn_from_totals(
    ann: &U512,
    nn_total_prod_calc: &U512,
    ann_total_sum_calc: &U512,
    d: &U512,
) -> Option<bool> {
    let inv_right = *d * *ann
        + vec![*d; N_TRADABLE_ASSETS + 1]
            .into_iter()
            .fold(U512::one(), |a, b| a * b)
            / *nn_total_prod_calc;
    let inv_left = *ann_total_sum_calc + *d;
    Some(inv_right >= inv_left)
}

pub fn calculate_invariant_error_sgn(
    x_calc: &U512,
    y_calc: &U512,
    d: &U512,
    nn: U512,
    ann: &U512,
) -> Option<bool> {
    let nn_total_prod_calc = nn * x_calc * y_calc;
    let ann_total_sum_calc = *ann * (x_calc + y_calc);
    calculate_invariant_error_sgn_from_totals(ann, &nn_total_prod_calc, &ann_total_sum_calc, d)
}

pub fn check_exact_invariant(
    quote_mult: &U512,
    tradable_base_before: &U512,
    tradable_quote_before: &U512,
    tradable_base_after: &U512,
    tradable_quote_after: &U512,
    d: &U512,
    nn: &U512,
    an2n: &U512,
) -> Option<bool> {
    let max_swap_err = U512::from(MAX_SWAP_ERROR);
    let an2n_nn = an2n - nn;
    let dn1 = vec![*d; N_TRADABLE_ASSETS + 1]
        .into_iter()
        .fold(U512::one(), |a, b| a * b);
    let total_prod_calc_before = tradable_base_before * tradable_quote_before;

    let alpha_before = an2n_nn * total_prod_calc_before;
    let beta_before = an2n * total_prod_calc_before * (tradable_base_before + tradable_quote_before);
    let total_prod_calc_after = tradable_base_after * tradable_quote_after;
    let alpha_after = an2n_nn * total_prod_calc_after;
    let beta_after = an2n * total_prod_calc_after * (tradable_base_after + tradable_quote_after);
    let max_quote_error = max_swap_err * quote_mult;
    if max_quote_error <= *tradable_quote_after {
        let total_prod_calc_after_shifter = tradable_base_after * (tradable_quote_after - max_quote_error);
        let alpha_after_shifted = an2n_nn * total_prod_calc_after_shifter;
        let beta_after_shifted = an2n
            * total_prod_calc_after_shifter
            * (tradable_base_after + (tradable_quote_after - max_quote_error));
        let check_result = dn1 + alpha_before * d >= beta_before
            && dn1 + alpha_after * d <= beta_after
            && dn1 + alpha_after_shifted * d >= beta_after_shifted;
        return Some(check_result);
    }
    None
}

pub fn calculate_context_values_list(prev_state: StablePoolT2T, new_state: StablePoolT2T) -> Option<U512> {
    let nn = U512::from(4);
    let unit = U512::from(1);
    let an2n_calc = U512::from(prev_state.an2n);
    let x_mult = U512::from(prev_state.multiplier_x);
    let y_mult = U512::from(prev_state.multiplier_y);

    let tradable_reserves_x0 = U512::from(prev_state.reserves_x.untag() - prev_state.treasury_x.untag());
    let tradable_reserves_y0 = U512::from(prev_state.reserves_y.untag() - prev_state.treasury_y.untag());

    let tradable_reserves_x1 = U512::from(new_state.reserves_x.untag() - new_state.treasury_x.untag());
    let tradable_reserves_y1 = U512::from(new_state.reserves_y.untag() - new_state.treasury_y.untag());

    let (base_init, quote_init, base_after_calc, quote, quote_delta, quote_mult, quote_lp_fee) =
        if tradable_reserves_x1 > tradable_reserves_x0 {
            (
                tradable_reserves_x0 * x_mult,
                tradable_reserves_y0 * y_mult,
                tradable_reserves_x1 * x_mult,
                tradable_reserves_y1,
                tradable_reserves_y0 - tradable_reserves_y1,
                y_mult,
                U512::from(*prev_state.lp_fee_y.numer()),
            )
        } else {
            (
                tradable_reserves_y0 * y_mult,
                tradable_reserves_x0 * x_mult,
                tradable_reserves_y1 * y_mult,
                tradable_reserves_x1,
                tradable_reserves_x0 - tradable_reserves_x1,
                x_mult,
                U512::from(*prev_state.lp_fee_x.numer()),
            )
        };
    let denom = U512::from(*prev_state.lp_fee_y.denom());

    let quote_no_lp_fees = quote - quote_delta * quote_lp_fee / (denom - quote_lp_fee) - unit;
    let quote_after_calc = quote_no_lp_fees * quote_mult;

    let mut inv = calculate_invariant(&base_init, &quote_init, &an2n_calc)?;

    let mut valid_inv = check_exact_invariant(
        &quote_mult,
        &base_init,
        &quote_init,
        &base_after_calc,
        &quote_after_calc,
        &inv,
        &nn,
        &an2n_calc,
    )?;

    while !valid_inv {
        inv += unit;
        valid_inv = check_exact_invariant(
            &quote_mult,
            &base_init,
            &quote_init,
            &base_after_calc,
            &quote_after_calc,
            &inv,
            &nn,
            &an2n_calc,
        )?
    }
    Some(inv)
}
pub fn calculate_invariant(x_calc: &U512, y_calc: &U512, an2n: &U512) -> Option<U512> {
    let unit = U512::from(1);
    let zero = U512::from(0);
    if *x_calc == zero || *y_calc == zero || *an2n == zero {
        return None;
    }
    let nn = U512::from(N_TRADABLE_ASSETS.pow(N_TRADABLE_ASSETS as u32));
    let n_calc = U512::from(N_TRADABLE_ASSETS);
    let ann = an2n / nn;
    let s = x_calc + y_calc;
    let p = x_calc * y_calc;

    let mut d = s;
    let mut abs_err = unit;
    while abs_err >= unit {
        let d_previous = d;
        let dn1 = vec![d_previous; N_TRADABLE_ASSETS + 1]
            .into_iter()
            .fold(U512::one(), |a, b| a * b);
        let d_p = dn1 / nn / p;
        let d_num = (ann * s + n_calc * d_p) * d_previous;
        let d_den = (ann - unit) * d_previous + (n_calc + unit) * d_p;
        d = d_num / d_den;
        abs_err = if d > d_previous {
            d - d_previous
        } else {
            d_previous - d
        };
    }

    let mut inv_err = calculate_invariant_error_sgn(x_calc, y_calc, &d, nn, &ann)?;

    let inv_err_upper = calculate_invariant_error_sgn(x_calc, y_calc, &(d + unit), nn, &ann)?;
    if !(inv_err && !inv_err_upper) {
        while !inv_err {
            d += unit;
            inv_err = calculate_invariant_error_sgn(x_calc, y_calc, &d, nn, &ann)?
        }
    }
    Some(d)
}

pub fn calculate_y_given_x(x: &BigNumber, d: &BigNumber, an2n: &BigNumber) -> BigNumber {
    let sqrt_degree = BigNumber::from(0.5);
    let n_2 = BigNumber::from(2);
    let n_4 = BigNumber::from(4);
    let n = BigNumber::from(N_TRADABLE_ASSETS);
    let nn = n.pow(&n.clone());
    let n2n = n.pow(&(n_2.clone() * n.clone()));

    let dn = d.pow(&n_2.clone());
    let dn1 = dn.clone() * d.clone();
    let d2n = d.pow(&(n_2.clone() * n.clone()));
    let a = an2n.clone() / n2n.clone();

    let x2 = x.pow(&n_2);
    let br_val = a.clone() * d.clone() * nn.clone() - d.clone() - a.clone() * nn.clone() * x.clone();
    let br = <u64>::try_from(br_val.value.to_int().value().unsigned_abs()).unwrap();
    let br_square = BigNumber::from(br as f64).pow(&n_2);
    let c = dn.clone()
        * ((br_square.clone() * x.clone() + n_4 * a.clone() * dn1.clone()) * x.clone() * n2n.clone()
            / d2n.clone())
        .pow(&sqrt_degree);

    (an2n.clone() * d.clone() * x.clone() - d.clone() * nn.clone() * x.clone() - an2n.clone() * x2.clone()
        + c)
        / (n_2.clone() * an2n * x)
}

pub fn calculate_safe_price_ratio_x_y_swap(
    target_price: &f64,
    d_value: &u128,
    x_initial: &u64,
    an2n_value: &u64,
    total_fee: &f64,
    alpha: &u64,
) -> (u64, u64) {
    // Relative price: out/input i.e. delta y / delta x:
    const PRICE_PREC: f64 = 1_000_000f64;
    let p = BigNumber::from(*target_price);
    let total_fee_mult = BigNumber::from(1f64 - total_fee);

    let n_2 = BigNumber::from(2);
    let an2n = BigNumber::from(*an2n_value as f64);

    let d = BigNumber::from(*d_value as f64);
    let dn1 = d.pow(&BigNumber::from(N_TRADABLE_ASSETS + 1));

    let b = dn1 + an2n.clone();

    let alpha = BigNumber::from(*alpha as f64);

    let mut x_f64: f64 = *x_initial as f64;
    let x_step = BigNumber::from(1);

    let mut add: f64 = 2.0;
    let mut counter = 0;
    let mut current_spot_price_val = 0f64;
    while !(current_spot_price_val >= *target_price
        && (current_spot_price_val - target_price).abs() < target_price / PRICE_PREC)
        && counter < 1000
    {
        counter += 1;
        let x = BigNumber::from(x_f64);
        let x2 = x.pow(&n_2);
        let y = calculate_y_given_x(&x, &d, &an2n);
        let y2 = y.pow(&n_2);

        let current_spot_price = ((x.clone() * (b.clone() + an2n.clone() * x.clone() * y2.clone()))
            / (y.clone() * (b.clone() + an2n.clone() * x2.clone() * y.clone())))
            / total_fee_mult.clone();

        current_spot_price_val = <f64>::try_from(current_spot_price.value.to_f64().value()).unwrap();

        let f = p.clone() - current_spot_price.clone();
        let x_1 = x.clone() + x_step.clone();
        let y_1 = calculate_y_given_x(&x_1, &d, &an2n);
        let y_12 = y_1.pow(&n_2);
        let f_1 = p.clone()
            - ((x_1.clone() * (b.clone() + an2n.clone() * x_1 * y_12))
                / (y_1.clone() * (b.clone() + an2n.clone() * x2.clone() * y_1.clone())))
                / total_fee_mult.clone();

        let f_x = (f_1.clone() - f.clone()) / x_step.clone();

        let add_value = alpha.clone() * f.clone() / f_x.clone();
        add = <f64>::try_from(add_value.value.to_f64().value()).unwrap();
        x_f64 -= add;
    }
    x_f64 += add;
    let y_safe = calculate_y_given_x(&BigNumber::from(x_f64), &d, &an2n);
    (
        x_f64 as u64,
        <f64>::try_from(y_safe.value.to_f64().value()).unwrap() as u64,
    )
}

#[cfg(test)]
mod test {
    use bignumber::BigNumber;
    use cml_crypto::ScriptHash;
    use num_rational::Ratio;
    use primitive_types::U512;

    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::AssetClass::Native;
    use spectrum_cardano_lib::{AssetClass, AssetName, TaggedAmount, TaggedAssetClass, Token};

    use crate::constants::MAX_LQ_CAP;
    use crate::data::order::{Base, Quote};
    use crate::data::stable_pool_t2t::{StablePoolT2T, StablePoolT2TVer};
    use crate::data::PoolId;
    use crate::pool_math::stable_pool_t2t_exact_math::{
        calc_stable_swap, calculate_context_values_list, calculate_invariant,
        calculate_safe_price_ratio_x_y_swap, calculate_y_given_x, check_exact_invariant,
    };

    fn gen_ada_token_pool(
        reserves_x: u64,
        x_decimals: u32,
        reserves_y: u64,
        y_decimals: u32,
        lp_fee_x: u64,
        lp_fee_y: u64,
        treasury_fee: u64,
        treasury_x: u64,
        treasury_y: u64,
        a: u64,
    ) -> StablePoolT2T {
        let an2n = a * 16;
        let (multiplier_x, multiplier_y) = if (x_decimals > y_decimals) {
            (1, 10_u32.pow(x_decimals - y_decimals))
        } else if (x_decimals < y_decimals) {
            (10_u32.pow(y_decimals - x_decimals), 1)
        } else {
            (1, 1)
        };
        let inv_before = calculate_invariant(
            &U512::from((reserves_x - treasury_x) * multiplier_x as u64),
            &U512::from((reserves_y - treasury_y) * multiplier_x as u64),
            &U512::from(an2n),
        )
        .unwrap();
        let liquidity = MAX_LQ_CAP - inv_before.as_u64();

        return StablePoolT2T {
            id: PoolId::from(Token(
                ScriptHash::from([
                    162, 206, 112, 95, 150, 240, 52, 167, 61, 102, 158, 92, 11, 47, 25, 41, 48, 224, 188,
                    211, 138, 203, 127, 107, 246, 89, 115, 157,
                ]),
                AssetName::from((
                    3,
                    [
                        110, 102, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0,
                    ],
                )),
            )),
            an2n: an2n, // constant
            reserves_x: TaggedAmount::new(reserves_x),
            multiplier_x: multiplier_x as u64,
            reserves_y: TaggedAmount::new(reserves_y),
            multiplier_y: multiplier_y as u64,
            liquidity: TaggedAmount::new(liquidity),
            asset_x: TaggedAssetClass::new(AssetClass::Native),
            asset_y: TaggedAssetClass::new(AssetClass::Token(Token(
                ScriptHash::from([
                    75, 52, 89, 253, 24, 161, 219, 171, 226, 7, 205, 25, 201, 149, 26, 159, 172, 159, 92, 15,
                    156, 56, 78, 61, 151, 239, 186, 38,
                ]),
                AssetName::from((
                    5,
                    [
                        116, 101, 115, 116, 67, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0, 0,
                    ],
                )),
            ))),
            asset_lq: TaggedAssetClass::new(AssetClass::Token(Token(
                ScriptHash::from([
                    114, 191, 27, 172, 195, 20, 1, 41, 111, 158, 228, 210, 254, 123, 132, 165, 36, 56, 38,
                    251, 3, 233, 206, 25, 51, 218, 254, 192,
                ]),
                AssetName::from((
                    2,
                    [
                        108, 113, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0,
                    ],
                )),
            ))),
            lp_fee_x: Ratio::new_raw(lp_fee_x, 100000),
            lp_fee_y: Ratio::new_raw(lp_fee_y, 100000),
            treasury_fee: Ratio::new_raw(treasury_fee, 100000),
            treasury_x: TaggedAmount::new(treasury_x),
            treasury_y: TaggedAmount::new(treasury_y),
            ver: StablePoolT2TVer::V1,
            marginal_cost: ExUnits {
                mem: 120000000,
                steps: 100000000000,
            },
        };
    }

    #[test]
    fn check_exact_invariant_test() {
        let base_mult = U512::from(1000);
        let quote_mult = U512::from(1);

        let tradable_quote_before = (U512::from(475000220u64) - U512::from(220000220u64)) * quote_mult;
        let tradable_base_before = (U512::from(343088u64) - U512::from(88088u64)) * base_mult;
        let tradable_base_after = (U512::from(390088u64) - U512::from(88088u64)) * base_mult;
        let tradable_quote_after_no_last_lp_fees = U512::from(208014916u64) * quote_mult;

        let d = U512::from(510000000u64);
        let nn = U512::from(4);
        let an2n = U512::from(300 * 16);
        assert_eq!(
            check_exact_invariant(
                &U512::from(1),
                &tradable_base_before,
                &tradable_quote_before,
                &tradable_base_after,
                &tradable_quote_after_no_last_lp_fees,
                &d,
                &nn,
                &an2n,
            ),
            Some(true)
        );
    }

    #[test]
    fn swap_test() {
        let reserves_x = TaggedAmount::<Base>::new(102434231u64);
        let reserves_y = TaggedAmount::<Quote>::new(3002434231u64);
        let base_amount = TaggedAmount::new(7110241u64);
        let an2n: u64 = 220 * 16;
        let quote_final = calc_stable_swap(
            TaggedAssetClass::new(Native),
            reserves_x,
            1,
            reserves_y,
            1,
            TaggedAssetClass::new(Native),
            base_amount,
            an2n,
        )
        .unwrap();
        assert_eq!(quote_final.untag(), 8790136)
    }

    #[test]
    fn test_calculate_context_values_list() {
        let lp_fee = 100u64;
        let tr_fee = 100u64;
        let a = 200;

        let reserves_x0 = 100000000u64;
        let reserves_y0 = 100000000u64;

        let reserves_x1 = 100100000u64;
        let reserves_y1 = 99900201u64;

        let prev = gen_ada_token_pool(reserves_x0, 0, reserves_y0, 0, lp_fee, lp_fee, tr_fee, 0, 0, a);
        let new = gen_ada_token_pool(reserves_x1, 0, reserves_y1, 0, lp_fee, lp_fee, tr_fee, 0, 100, a);
        let inv = calculate_context_values_list(prev, new).unwrap();
        assert_eq!(U512::from(200000000), inv);

        let lp_fee = 20000u64;
        let tr_fee = 50000u64;
        let a = 300;

        let reserves_x0 = 475000220u64;
        let reserves_y0 = 343088u64;

        let reserves_x1 = 460904695u64;
        let reserves_y1 = 390088u64;

        let prev = gen_ada_token_pool(
            reserves_x0,
            6,
            reserves_y0,
            3,
            lp_fee,
            lp_fee,
            tr_fee,
            220000220,
            88088,
            a,
        );
        let new = gen_ada_token_pool(
            reserves_x1,
            6,
            reserves_y1,
            3,
            lp_fee,
            lp_fee,
            tr_fee,
            243492762,
            88088,
            a,
        );
        let inv = calculate_context_values_list(prev, new).unwrap();
        assert_eq!(U512::from(510000000), inv);
    }
    #[test]
    fn calculate_y_given_x_test() {
        let x = BigNumber::from(1_000_000 as f64);
        let d = BigNumber::from(2_000_000 as f64);
        let an2n = BigNumber::from((200 * 16) as f64);

        let y = calculate_y_given_x(&x, &d, &an2n);
        assert_eq!(
            <f64>::try_from(y.value.to_f64().value()).unwrap() as u64,
            1_000_000u64
        );
    }
    #[test]
    fn calculate_safe_price_ratio_test() {
        let (x_safe, y_safe) =
            calculate_safe_price_ratio_x_y_swap(&1.0, &2089992, &990000, &(200 * 16), &0f64, &200);
        assert_eq!(x_safe, 1044995);
        assert_eq!(y_safe, 1044996);
    }
}
