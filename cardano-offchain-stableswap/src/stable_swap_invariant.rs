use primitive_types::U512;

/// Calculates StableSwap invariant error.
/// StableSwap invariant expression can be represented as: \
/// \
/// `d * Ann + d^(n + 1) / (nn * Prod(reserves)) = Ann * Sum(reserves) + d`,
/// \
/// \
/// where:
/// * `d` - Invariant value;
/// * `n` - Number of tradable assets;
/// * `nn = n^n`;
/// * `Ann = ampl_coefficient * nn`;
/// * `reserves = vec![asset_{i}_balance; n]`.
/// \
/// \
/// Since we are working with integers the left side of the expression is not equal to the right in most cases.
/// "Invariant error" is the absolute difference between the left and right sides of the expression above.
///
/// # Arguments
/// * `n` - Number of tradable assets;
/// * `ann` - Amplification coefficient of the StableSwap invariant multiplied by `n^n`;
/// * `nn_total_prod_calc` - Production of pool reserves balances multiplied by `n^n`;
/// * `ann_total_sum_calc` - Sum of pool reserves balances multiplied by `ann`;
/// * `d` - Value of the StableSwap invariant.
///
/// # Outputs
/// * `inv_abs_err` - "Invariant error".
pub fn calculate_invariant_error_from_totals(
    n: &u32,
    ann: &U512,
    nn_total_prod_calc: &U512,
    ann_total_sum_calc: &U512,
    d: &U512,
) -> U512 {
    let inv_right = *d * *ann
        + vec![*d; usize::try_from(*n + 1).unwrap()]
            .iter()
            .copied()
            .reduce(|a, b| a * b)
            .unwrap()
            / *nn_total_prod_calc;
    let inv_left = *ann_total_sum_calc + *d;

    if inv_right > inv_left {
        inv_right - inv_left
    } else {
        inv_left - inv_right
    }
}

/// Calculates StableSwap invariant error from pure reserves balances with common precision.
///
/// # Arguments
/// * `n` - Number of tradable assets;
/// * `ann` - Amplification coefficient of the StableSwap invariant multiplied by `n^n`;
/// * `balances_calc` - Reserves balances with common precision;
/// * `d` - Value of the StableSwap invariant.
///
/// # Outputs
/// * `inv_abs_err` - "Invariant error".
pub fn calculate_invariant_error_from_balances(
    n: &u32,
    ann: &U512,
    balances_calc: &Vec<U512>,
    d: &U512,
) -> U512 {
    // Note: input reserves must be reduced to common precision:
    let nn = U512::from(n.pow(*n));
    let nn_total_prod_calc = nn * balances_calc.iter().copied().reduce(|a, b| a * b).unwrap();
    let ann_total_sum_calc = *ann * balances_calc.iter().copied().reduce(|a, b| a + b).unwrap();

    calculate_invariant_error_from_totals(n, ann, &nn_total_prod_calc, &ann_total_sum_calc, d)
}

/// Checks if StableSwap invariant function is at it's extremum (minimum) point with the given values.
/// \
/// Note: input balances must be reduced to a common denominator (precision).
/// # Arguments
/// * `n` - Number of tradable assets;
/// * `ann` - Amplification coefficient of the StableSwap invariant multiplied by `n^n`;
/// * `balances_calc` - Reserves balances with common precision;
/// * `d` - Value of the StableSwap invariant.
///
/// # Outputs
///
/// * `is_extremum` - boolean.
pub fn check_invariant_extremum(n: &u32, ann: &U512, balances_calc: &Vec<U512>, d: &U512) -> bool {
    let unit = U512::from(1);
    let err_eq = calculate_invariant_error_from_balances(n, ann, balances_calc, d);

    let err_left = calculate_invariant_error_from_balances(n, ann, balances_calc, &(*d - unit));

    let err_right = calculate_invariant_error_from_balances(n, ann, balances_calc, &(*d + unit));
    err_left >= err_eq && err_right >= err_eq
}

/// StableSwap invariant value numerical calculation procedure.
/// \
/// Note: input balances must be reduced to a common denominator (precision).
/// \
/// Function solves the equation
/// \
/// \
/// `d * Ann + d^(n + 1) / (nn * Prod(reserves)) = Ann * Sum(reserves) + d`
/// \
/// \
/// numerically relative to `d` while all other parameters are known.
/// # Arguments
/// * `balances` -Reserves balances;
/// * `n` - Number of tradable assets;
/// * `ampl_coefficient` - Amplification coefficient of the StableSwap invariant.
///
/// # Outputs
/// * `d` - value of the StableSwap invariant.
pub fn calculate_invariant(balances: &Vec<U512>, n_assets: &u32, ampl_coefficient: &u32) -> U512 {
    assert_eq!(balances.len(), (*n_assets).try_into().unwrap());
    // Constants commonly used in calculations:
    let unit = U512::from(1);
    let n = U512::from(*n_assets);
    let nn = U512::from(n_assets.pow(*n_assets));
    let ann = U512::from(*ampl_coefficient) * nn;

    // Invariant calculation with the Newton-Raphson method:
    let s = balances.iter().copied().reduce(|a, b| a + b).unwrap();
    let p = balances.iter().copied().reduce(|a, b| a * b).unwrap();

    let mut d = s.clone();
    let mut abs_d_error = d;
    while abs_d_error > unit {
        let d_previous = d;
        let d_p = vec![d_previous; usize::try_from(*n_assets + 1).unwrap()]
            .iter()
            .copied()
            .reduce(|a, b| a * b)
            .unwrap()
            / nn
            / p;
        d = (ann * s + n * d_p) * d_previous / ((ann - unit) * d_previous + (n + unit) * d_p);
        abs_d_error = if d >= d_previous {
            d - d_previous
        } else {
            d_previous - d
        }
    }

    // Fine tuning of the calculated value:
    let nn_total_prod_calc = nn * p;
    let ann_total_sum_calc = ann * s;
    let mut inv_err =
        calculate_invariant_error_from_totals(n_assets, &ann, &nn_total_prod_calc, &ann_total_sum_calc, &d);
    let inv_err_upper = calculate_invariant_error_from_totals(
        n_assets,
        &ann,
        &nn_total_prod_calc,
        &ann_total_sum_calc,
        &(d + unit),
    );
    let inv_err_lower = calculate_invariant_error_from_totals(
        n_assets,
        &ann,
        &nn_total_prod_calc,
        &ann_total_sum_calc,
        &(d - unit),
    );

    if !((inv_err < inv_err_upper) && (inv_err < inv_err_lower)) {
        let mut inv_err_previous = inv_err + unit;

        while inv_err < inv_err_previous {
            inv_err_previous = inv_err;
            d = d + unit;
            inv_err = calculate_invariant_error_from_totals(
                n_assets,
                &ann,
                &nn_total_prod_calc,
                &ann_total_sum_calc,
                &d,
            );
        }

        d = d - unit
    }
    d
}

mod test {
    use primitive_types::U512;
    use rand::seq::SliceRandom;
    use rand::Rng;

    use crate::stable_swap_invariant::{
        calculate_invariant, calculate_invariant_error_from_balances, calculate_invariant_error_from_totals,
    };

    #[test]
    fn calculate_invariant_error_from_totals_test() {
        let n_assets_set: Vec<u32> = vec![2, 3, 4];
        let mut rng = rand::thread_rng();
        let mut err_vec = Vec::new();
        for _ in 0..100 {
            let a: U512 = U512::from(rng.gen_range(1..2000));
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let nn = U512::from(n.pow(n));
            let ann = a * nn;
            let balance: u64 = rng.gen();
            let balances_calc = vec![U512::from(balance); n as usize];
            let nn_total_prod_calc = nn * balances_calc.iter().copied().reduce(|a, b| a * b).unwrap();
            let ann_total_sum_calc = ann * balances_calc.iter().copied().reduce(|a, b| a + b).unwrap();
            let inv: U512 = U512::from(n) * U512::from(balance);

            let err = calculate_invariant_error_from_totals(
                &n,
                &ann,
                &nn_total_prod_calc,
                &ann_total_sum_calc,
                &inv,
            );
            err_vec.push(err);
            assert_eq!(err, U512::from(0));
        }
    }

    #[test]
    fn calculate_invariant_error_from_balances_test() {
        let n_assets_set: Vec<u32> = vec![2, 3, 4];
        let mut rng = rand::thread_rng();
        for _ in 0..100 {
            let a: U512 = U512::from(rng.gen_range(1..2000));
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let nn = U512::from(n.pow(n));
            let ann = a * nn;
            let balance: u64 = rng.gen();
            let balances_calc = vec![U512::from(balance); n as usize];
            let inv: U512 = U512::from(n) * U512::from(balance);
            let err = calculate_invariant_error_from_balances(&n, &ann, &balances_calc, &inv);
            assert_eq!(err, U512::from(0))
        }
    }

    #[test]
    fn calculate_invariant_test() {
        let n_assets_set: Vec<u32> = vec![2, 3, 4];
        let mut rng = rand::thread_rng();
        for _ in 0..100 {
            let a = rng.gen_range(1..2000);
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let balance: u64 = rng.gen();
            let balances_calc = vec![U512::from(balance); n as usize];
            let inv = calculate_invariant(&balances_calc, &n, &a);
            assert_eq!(inv, U512::from(n) * U512::from(balance))
        }
    }
}
