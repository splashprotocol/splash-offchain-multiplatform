use primitive_types::U512;

use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};

use crate::data::order::{Base, Quote};
use crate::data::stable_pool_t2t::StablePoolT2T;

const MAX_SWAP_ERROR: u64 = 2;
const N_TRADABLE_ASSETS: u64 = 2;

pub fn calc_stable_swap<X, Y>(
    asset_x: TaggedAssetClass<X>,
    reserves_x: TaggedAmount<X>,
    x_mult: u64,
    reserves_y: TaggedAmount<Y>,
    y_mult: u64,
    base_asset: TaggedAssetClass<Base>,
    base_amount: TaggedAmount<Base>,
    an2n: u64,
) -> TaggedAmount<Quote> {
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

    let d = calculate_invariant(&base_initial, &quote_calc, &an2n_calc);
    let b = s + d / ann;
    let dn1 = vec![d; usize::try_from(N_TRADABLE_ASSETS + 1).unwrap()]
        .iter()
        .copied()
        .reduce(|a, b| a * b)
        .unwrap();
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

    let d_new = calculate_invariant(&base_calc, &asset_to, &an2n_calc);
    let d_after = if { d_new > d } { d_new.clone() } else { d.clone() };

    let mut valid_inv = check_exact_invariant(
        &U512::from(quote_mult),
        &base_initial,
        &asset_to_initial,
        &base_calc,
        &asset_to,
        &d_after,
        &nn,
        &an2n_calc,
    );
    let mut counter = 0;

    if !valid_inv {
        while !valid_inv && counter < 255 {
            valid_inv = check_exact_invariant(
                &U512::from(quote_mult),
                &base_initial,
                &asset_to_initial,
                &base_calc,
                &asset_to,
                &d_after,
                &nn,
                &an2n_calc,
            );
            asset_to += unit;
            counter += 1;
        }
    }

    assert_eq!(
        check_exact_invariant(
            &U512::from(quote_mult),
            &base_initial,
            &asset_to_initial,
            &base_calc,
            &asset_to,
            &d_after,
            &nn,
            &an2n_calc
        ),
        true
    );

    let quote_amount_pure_delta = asset_to_initial - asset_to;

    TaggedAmount::new((quote_amount_pure_delta / quote_mult).as_u64())
}

pub fn calculate_invariant_error_sgn_from_totals(
    ann: &U512,
    nn_total_prod_calc: &U512,
    ann_total_sum_calc: &U512,
    d: &U512,
) -> bool {
    let inv_right = *d * *ann
        + vec![*d; usize::try_from(N_TRADABLE_ASSETS + 1).unwrap()]
            .iter()
            .copied()
            .reduce(|a, b| a * b)
            .unwrap()
            / *nn_total_prod_calc;
    let inv_left = *ann_total_sum_calc + *d;
    inv_right >= inv_left
}

pub fn calculate_invariant_error_sgn(x_calc: &U512, y_calc: &U512, d: &U512, nn: U512, ann: &U512) -> bool {
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
) -> bool {
    let max_swap_err = U512::from(MAX_SWAP_ERROR);
    let an2n_nn = an2n - nn;
    let dn1 = vec![*d; usize::try_from(N_TRADABLE_ASSETS + 1).unwrap()]
        .iter()
        .copied()
        .reduce(|a, b| a * b)
        .unwrap();
    let total_prod_calc_before = tradable_base_before * tradable_quote_before;

    let alpha_before = an2n_nn * total_prod_calc_before;
    let beta_before = an2n * total_prod_calc_before * (tradable_base_before + tradable_quote_before);
    let total_prod_calc_after = tradable_base_after * tradable_quote_after;
    let alpha_after = an2n_nn * total_prod_calc_after;
    let beta_after = an2n * total_prod_calc_after * (tradable_base_after + tradable_quote_after);
    let total_prod_calc_after_shifter =
        tradable_base_after * (tradable_quote_after - max_swap_err * quote_mult);
    let alpha_after_shifted = an2n_nn * total_prod_calc_after_shifter;
    let beta_after_shifted = an2n
        * total_prod_calc_after_shifter
        * (tradable_base_after + (tradable_quote_after - max_swap_err * quote_mult));
    dn1 + alpha_before * d >= beta_before
        && dn1 + alpha_after * d <= beta_after
        && dn1 + alpha_after_shifted * d >= beta_after_shifted
}

pub fn calculate_context_values_list(
    prev_state: StablePoolT2T,
    new_state: StablePoolT2T
) -> U512 {
    //todo: implement
    unimplemented!()
}

pub fn calculate_invariant(x_calc: &U512, y_calc: &U512, an2n: &U512) -> U512 {
    let nn = U512::from(N_TRADABLE_ASSETS.pow(N_TRADABLE_ASSETS as u32));
    let unit = U512::from(1);
    let n_calc = U512::from(N_TRADABLE_ASSETS);
    let ann = an2n / nn;
    let s = x_calc + y_calc;
    let p = x_calc * y_calc;

    let mut d_previous = U512::zero();
    let mut d = s.clone();
    let mut abs_err = unit;
    while abs_err >= unit {
        d_previous = d;
        let dn1 = vec![d_previous; usize::try_from(N_TRADABLE_ASSETS + 1).unwrap()]
            .iter()
            .copied()
            .reduce(|a, b| a * b)
            .unwrap();
        let d_p = dn1 / nn / p;
        d = (ann * s + n_calc * d_p) * d_previous / ((ann - unit) * d_previous + (n_calc + unit) * d_p);
        abs_err = if d > d_previous {
            d - d_previous
        } else {
            d_previous - d
        };
    }

    let mut inv_err = calculate_invariant_error_sgn(x_calc, y_calc, &d, nn, &ann);

    let inv_err_upper = calculate_invariant_error_sgn(x_calc, y_calc, &(d + unit), nn, &ann);
    if !(inv_err && !inv_err_upper) {
        while !inv_err {
            d += unit;
            inv_err = calculate_invariant_error_sgn(x_calc, y_calc, &d, nn, &ann)
        }
    }
    return d + unit;
}

#[cfg(test)]
mod test {
    use num_rational::Ratio;

    use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
    use spectrum_cardano_lib::AssetClass::Native;

    use crate::data::order::{Base, Quote};
    use crate::pool_math::stable_pool_t2t_exact_math::calc_stable_swap;

    #[test]
    fn swap_test() {
        let reserves_x = TaggedAmount::<Base>::new(102434231u64);
        let reserves_y = TaggedAmount::<Quote>::new(3002434231u64);
        let base_amount = TaggedAmount::new(7110241u64);
        let pool_fee_x = Ratio::new_raw(7000, 100000);
        let pool_fee_y = Ratio::new_raw(7000, 100000);
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
        );
        assert_eq!(quote_final.untag(), 8790133)
    }
}
