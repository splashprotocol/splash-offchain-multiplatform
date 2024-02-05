use crate::crypto_pools_amm_actions::PRECISION;
use crate::curve_crypto_invariant::calculate_crypto_exchange;
use crate::math_utils::nonzero_prod;
use primitive_types::U512;
use std::f32;

/// Calculates new price vector.
///
/// # Arguments
/// * `balances_calc` - Price-scaled balances with common precision;
/// * `n` - Number of tradable assets;
/// * `a` - Amplification coefficient of the CurveCrypto invariant;
/// * `gamma_num` - Distance coefficient numerator;
/// * `gamma_denom` - Distance coefficient denominator.
///
/// # Outputs
/// * `price_vector_new` - New price vector (relative to 0-th pool's asset, i.e. it is excluded from the vector).
pub fn calculate_price_vector(
    balances_calc: &Vec<U512>,
    n: &u32,
    a: &U512,
    gamma_num: &U512,
    gamma_denom: &U512,
) -> Vec<U512> {
    let base_amount = U512::from(PRECISION);
    let mut new_price_vec = vec![];
    for j in 1..*n {
        let quote_j_amount_final = calculate_crypto_exchange(
            &0,
            &(j as usize),
            &base_amount,
            &balances_calc,
            &n,
            &a,
            &gamma_num,
            &gamma_denom,
        );
        new_price_vec.push(balances_calc[j as usize] - quote_j_amount_final);
    }
    new_price_vec
}

/// Main price re-pegging function.
///
/// # Arguments
/// * `n` - Number of tradable assets;
/// * `balances_calc` - Price-scaled balances with common precision;
/// * `supply_lp` - Number of circulating LP-tokens;
/// * `price_vec` - Price vector;
/// * `d` - Value of the CurveCrypto invariant;
/// * `a` - Amplification coefficient of the CurveCrypto invariant;
/// * `gamma_num` - Distance coefficient numerator;
/// * `gamma_denom` - Distance coefficient denominator.
/// * `x_cp_profit_old` - Previous value of constant-product profit;
/// * `virtual_price_old` - Previous value of the virtual price.
///
/// # Outputs
/// * `price_vec_new` - New price vector;
/// * `x_cp_profit_new` - New value of constant-product profit;
/// * `virtual_price_new` - New value of the virtual price.

pub fn tweak_price(
    n: &u32,
    balances_calc: &Vec<U512>,
    supply_lp: &U512,
    price_vec: &Vec<U512>,
    d: &U512,
    a: &U512,
    gamma_num: &U512,
    gamma_denom: &U512,
    x_cp_profit_old: &U512,
    virtual_price_old: &U512,
) -> (Vec<U512>, U512, U512) {
    let nn = U512::from(n.pow(*n));
    let dn = vec![*d; usize::try_from(*n).unwrap()]
        .iter()
        .copied()
        .reduce(|x, y| x * y)
        .unwrap();
    let x_cp = calculate_x_cp(&n, &nn, &dn, &balances_calc);
    let virtual_price_new = x_cp / supply_lp;
    let x_cp_profit_new = x_cp_profit_old * virtual_price_new / virtual_price_old;

    let price_vec_new = if virtual_price_new * U512::from(2) > x_cp_profit_new + U512::from(2) {
        calculate_price_vector(balances_calc, n, a, &gamma_num, &gamma_denom)
    } else {
        (*price_vec.clone()).to_vec()
    };

    (price_vec_new, x_cp_profit_new, virtual_price_new)
}

/// Calculates constant product profit value.
///
/// # Arguments
/// * `n` - Number of tradable assets;
/// * `nn` - `n`^`n`;
/// * `dn` - `d`^`n`, where `d` is current value of the CurveCrypto invariant;
/// * `balances_calc` - Price-scaled balances with common precision;
///
/// # Outputs
/// * `x_cp` - profit value.
pub fn calculate_x_cp(n: &u32, nn: &U512, dn: &U512, balances_calc: &Vec<U512>) -> U512 {
    // Constants commonly used in calculations:
    let unit = U512::from(1);
    let n_calc = U512::from(*n);

    let p = nonzero_prod(&balances_calc);

    let c = dn / (nn * p);
    let mut abs_error = p;

    // Solve numerically:
    let x_cp_init = balances_calc.iter().fold(unit, |a, b| a.max(*b));
    let mut x_cp = x_cp_init;

    while abs_error > unit {
        let x_cp_previous = x_cp;
        let x_cp_previous_n1 = vec![x_cp_previous; usize::try_from(n - 1).unwrap()]
            .iter()
            .copied()
            .reduce(|x, y| x * y)
            .unwrap();

        let x_cp_n = x_cp_previous_n1 * x_cp_previous;
        let f_value = if x_cp_n > c { x_cp_n - c } else { c - x_cp };
        x_cp = if x_cp_n > c {
            x_cp_previous - f_value / (n_calc * x_cp_previous_n1)
        } else {
            x_cp_previous + f_value / (n_calc * x_cp_previous_n1)
        };
        abs_error = if x_cp >= x_cp_previous {
            x_cp - x_cp_previous
        } else {
            x_cp_previous - x_cp
        };
    }
    x_cp
}

/// Calculates price exponential moving average (EMA).
///
/// # Arguments
/// * `blocks_passed` - Number of blocks passed since last price update;
/// * `oracle_price` - Current price from the oracle;
/// * `last_price` - Last price from the pool data (last adjustment);
/// * `blocks_half` - half-period of the EMA exponent;
///
/// # Outputs
/// * `new_price` - profit value.
pub fn calculate_price_moving_average(
    blocks_passed: &u32,
    oracle_price: &u32,
    last_price: &u32,
    blocks_half: &u32,
) -> u32 {
    let alpha = 1f32 / f32::powf(2f32, *blocks_passed as f32 / *blocks_half as f32);

    (*oracle_price as f32 * (1f32 - alpha) + alpha * *last_price as f32) as u32
}

#[cfg(test)]
mod test {
    use crate::repegging_utils::{calculate_price_moving_average, calculate_price_vector, calculate_x_cp};
    use primitive_types::U512;
    use rand::prelude::SliceRandom;
    use rand::Rng;

    #[test]
    fn calculate_x_cp_test() {
        // Constants:
        let n = 3u32;
        let nn = U512::from(n.pow(n));
        // Balances:
        let balances_calc = vec![U512::from(2u32), U512::from(2124u32), U512::from(7542221u32)];
        let d = U512::from(u64::MAX);
        let dn = vec![d; usize::try_from(n).unwrap()]
            .iter()
            .copied()
            .reduce(|x, y| x * y)
            .unwrap();
        let x_cp = calculate_x_cp(&n, &nn, &dn, &balances_calc);
        assert_eq!(x_cp, U512::from(1935993435902969u64));
    }

    #[test]
    fn calculate_price_moving_average_test() {
        let blocks_passed = 34;
        let oracle_price = 100;
        let current_price = 200;
        let blocks_half = 20;
        let new_price =
            calculate_price_moving_average(&blocks_passed, &oracle_price, &current_price, &blocks_half);
        assert_eq!(new_price, 130u32);
    }

    #[test]
    fn calculate_price_vector_test() {
        let n_assets_set: Vec<u32> = vec![2, 3, 4];
        let mut rng = rand::thread_rng();
        for _ in 0..1 {
            let a: U512 = U512::from(rng.gen_range(1..10000));
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let gamma_num = U512::from(rng.gen_range(1..10_000_000));
            let gamma_denom = U512::from(1_000_000_000_000u64);
            // Reserves:
            let balance: u64 = rng.gen();
            let reserves = vec![U512::from(balance); n as usize];

            calculate_price_vector(&reserves, &n, &a, &gamma_num, &gamma_denom);
        }
    }
}
