use primitive_types::U512;

/// Calculates new fee numerator according to the current balances ratio in the pool.
///
/// # Arguments
/// * `n` - Number of tradable assets;
/// * `nn` - `n`^`n`;
/// * `s` - Sum of the reserve balances;
/// * `p` - Productions of the reserve balances;
/// * `fee_gamma_num` - Adjusts how fast the fee increases from `f_mid_num` to `f_out_num`;
/// * `f_mid_num` - Fee when the pool is maximally balanced;
/// * `f_out_num` - Fee when the pool is imbalanced;
/// * `denom` - Common denominator of calculations.
///
/// # Outputs
/// * `new_swap_fee_num` - New numerator of the swap fee.
pub fn calculate_new_fee_num(
    n: &u32,
    s: &U512,
    p: &U512,
    fee_gamma_num: &U512,
    fee_mid_num: &U512,
    fee_out_num: &U512,
    denom: &U512,
) -> U512 {
    let nn = U512::from(n.pow(*n));
    let sn = vec![*s; usize::try_from(*n).unwrap()]
        .iter()
        .copied()
        .reduce(|x, y| x * y)
        .unwrap();

    let g_denom = fee_gamma_num + denom - p * denom * nn / sn;

    (fee_gamma_num * fee_mid_num + (g_denom - fee_gamma_num) * fee_out_num) / g_denom
}

/// Calculates fee numerator for the imbalances liquidity action according to the current assets ratio in the pool.
///
/// # Arguments
/// * `swap_fee_num` - Numerator of the swap fee.
/// * `balances_calc` - Price-scaled balances with common precision.
///
/// # Outputs
/// * `lp_fee_num` - New numerator of the liquidity action fee.
pub fn calculate_lp_token_fee(fee_num: &U512, balances: &Vec<U512>) -> U512 {
    let n = U512::from(balances.len());
    let s = balances.iter().copied().reduce(|x, y| x + y).unwrap();
    let avg = s / n;

    let diff_vec = balances
        .iter()
        .enumerate()
        .map(|(k, _)| {
            if avg > balances[k] {
                avg - balances[k]
            } else {
                balances[k] - avg
            }
        })
        .collect::<Vec<U512>>();

    let diff_sum = diff_vec.iter().copied().reduce(|x, y| x + y).unwrap();
    fee_num * diff_sum / s
}

#[cfg(test)]
mod test {
    use crate::fees_utils::calculate_new_fee_num;
    use primitive_types::U512;
    use rand::prelude::SliceRandom;
    use rand::Rng;

    #[test]
    fn calculate_new_fee_gamma_num_test() {
        let n_assets_set: Vec<u32> = vec![2, 3, 4];
        let mut rng = rand::thread_rng();
        let denom_u64 = 10_000;

        for _ in 0..10000 {
            // Constants:
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let f_mid_num_u64 = rng.gen_range(1..denom_u64 / 2);
            let f_out_num_u64 = rng.gen_range(denom_u64 / 2..denom_u64);
            let fee_gamma_num = U512::from(rng.gen_range(10..200));

            // Balances:
            let balance: u64 = rng.gen();
            let balances_calc_equal = vec![U512::from(balance); n as usize];
            let s_eq = balances_calc_equal.iter().copied().reduce(|x, y| x + y).unwrap();
            let p_eq = balances_calc_equal.iter().copied().reduce(|x, y| x * y).unwrap();

            let mut balances_calc = Vec::new();
            for _ in 0..n {
                balances_calc.push(U512::from(rng.gen::<u64>()))
            }

            let new_gamma_num_eq = calculate_new_fee_num(
                &n,
                &s_eq,
                &p_eq,
                &fee_gamma_num,
                &U512::from(f_mid_num_u64),
                &U512::from(f_out_num_u64),
                &U512::from(denom_u64),
            );
            assert_eq!(new_gamma_num_eq.as_u64(), f_mid_num_u64);

            let s = balances_calc.iter().copied().reduce(|x, y| x + y).unwrap();
            let p = balances_calc.iter().copied().reduce(|x, y| x * y).unwrap();

            let new_gamma_num = calculate_new_fee_num(
                &n,
                &s,
                &p,
                &fee_gamma_num,
                &U512::from(f_mid_num_u64),
                &U512::from(f_out_num_u64),
                &U512::from(denom_u64),
            );
            assert!(new_gamma_num.as_u64() > f_mid_num_u64 && new_gamma_num.as_u64() < f_out_num_u64);
        }
    }
}
