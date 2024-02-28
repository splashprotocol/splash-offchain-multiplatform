use primitive_types::{U512};

use crate::stable_swap_invariant::{
    calculate_invariant, calculate_invariant_error_from_balances, check_invariant_extremum,
    check_invariant_extremum_for_asset,
};

pub const LP_EMISSION: u128 = u128::MAX;
pub const DENOM: u64 = 10_000;

/// Calculates valid transition of the StablePool reserves in the arbitrary Deposit/Redeem actions.
///
/// # Arguments
///
/// * `reserves_before` - Total reserves of the pool before the action;
/// * `reserves_after` - Total reserves of the pool after the action;
/// * `lp_amount_before` - Liquidity reserves of the pool before the action;
/// * `reserves_tokens_decimals` - Decimals of the tradable tokens;
/// * `collected_protocol_fees` - Total collected protocol fees before the action (from pool.datum);
/// * `swap_fee_num` - Numerator of the swap fee;
/// * `protocol_fee_num` - Numerator of the protocol's fee share;
/// * `ampl_coefficient` - Amplification coefficient of the StableSwap invariant (from pool.datum);
/// * `n_assets` - Number of tradable tokens in the pool.
///
/// # Outputs
///
/// * `collected_protocol_fees_final` - Total collected protocol fees after the action (put into pool.datum);
/// * `final_lp_amount` - Final amount of the LP tokens in the pool after the action;
/// * `d1` - Native invariant value (put into pool.datum);
/// * `d2` - Invariant value (put into pool.datum);
/// * `relevant_delta_lp_amount` - Relevant delta amount of the LP tokens.
pub fn liquidity_action(
    reserves_before: &Vec<U512>,
    reserves_after: &Vec<U512>,
    lp_amount_before: &U512,
    reserves_tokens_decimals: &Vec<U512>,
    collected_protocol_fees: &Vec<U512>,
    swap_fee_num: &u32,
    protocol_fee_num: &u32,
    ampl_coefficient: &u32,
    n_assets: &u32,
) -> (Vec<U512>, U512, U512, U512, U512) {
    assert_eq!(*n_assets, reserves_before.len() as u32);
    assert_eq!(*n_assets, reserves_after.len() as u32);
    assert_eq!(*n_assets, reserves_tokens_decimals.len() as u32);
    assert_eq!(*n_assets, collected_protocol_fees.len() as u32);
    // Calculate balances without collected protocol fees:
    let balances_before_no_fees = reserves_before
        .iter()
        .enumerate()
        .map(|(k, &x)| (x - collected_protocol_fees[k]))
        .collect::<Vec<U512>>();

    let mut balances_after_no_fees = reserves_after
        .iter()
        .enumerate()
        .map(|(k, &x)| (x - collected_protocol_fees[k]))
        .collect::<Vec<U512>>();

    // Convert to the equal precision for calculations:
    let precision = U512::from(reserves_tokens_decimals.iter().max().unwrap().0.as_slice()[0]);

    let balances_before_calc = balances_before_no_fees
        .iter()
        .enumerate()
        .map(|(k, &x)| x * precision / reserves_tokens_decimals[k])
        .collect::<Vec<U512>>();

    let balances_after_calc = balances_after_no_fees
        .iter()
        .enumerate()
        .map(|(k, &x)| x * precision / reserves_tokens_decimals[k])
        .collect::<Vec<U512>>();

    // Calculate initial (d0) and output (d1 aka native) values of the StableSwap invariant:
    let d0 = calculate_invariant(&balances_before_calc, n_assets, ampl_coefficient);
    let d1 = calculate_invariant(&balances_after_calc, n_assets, ampl_coefficient);

    // Calculate the ideal balances for the calculated output (d1) invariant:
    let balances_ideal = balances_before_no_fees
        .iter()
        .map(|x| d1 * *x / d0)
        .collect::<Vec<_>>();

    // Take commissions from the difference from the ideal distribution of balances:
    let mut difference = Vec::new();
    for asset_amounts in balances_ideal.iter().zip(balances_after_no_fees.iter_mut()) {
        let (ideal, real) = asset_amounts;
        let abs_diff = if ideal > real {
            *ideal - *real
        } else {
            *real - *ideal
        };
        difference.push(abs_diff)
    }

    // Calculate fees in the native units of the tradable assets:
    let denom = U512::from(DENOM);
    let swap_fee = U512::from((swap_fee_num * n_assets) / (4 * (n_assets - 1)));
    let protocol_fee = U512::from((protocol_fee_num * n_assets) / (4 * (n_assets - 1)));

    let lp_fees_native = difference
        .iter()
        .map(|&x| x * swap_fee / denom)
        .collect::<Vec<_>>();

    let protocol_fees_native = difference
        .iter()
        .map(|&x| x * protocol_fee / denom)
        .collect::<Vec<_>>();

    // Calculate final tradable balances with fees applied:
    let mut final_tradable_balances = Vec::new();

    for (a, b) in balances_after_no_fees.iter().zip(protocol_fees_native.iter()) {
        assert!(a > b);
        let new_balance = { *a - *b };
        final_tradable_balances.push(new_balance)
    }

    let final_tradable_balances_calc = final_tradable_balances
        .iter()
        .enumerate()
        .map(|(k, &x)| x * precision / reserves_tokens_decimals[k])
        .collect::<Vec<U512>>();
    // Take LP fees into account:
    let mut valid_lp_calc_balances = Vec::new();

    for (a, b) in final_tradable_balances.iter().zip(lp_fees_native.iter()) {
        assert!(a > b);
        let new_balance: U512 = { *a - *b };
        valid_lp_calc_balances.push(new_balance)
    }
    let valid_lp_calc_balances_calc = valid_lp_calc_balances
        .iter()
        .enumerate()
        .map(|(k, &x)| x * precision / reserves_tokens_decimals[k])
        .collect::<Vec<U512>>();

    let d2_ = calculate_invariant(&valid_lp_calc_balances_calc, n_assets, ampl_coefficient);

    // Calculate final value of the invariant:
    let d2 = calculate_invariant(&final_tradable_balances_calc, n_assets, ampl_coefficient);

    // Calculate final collected protocol fees:
    let collected_protocol_fees_final = collected_protocol_fees
        .iter()
        .enumerate()
        .map(|(k, &x)| x + protocol_fees_native[k])
        .collect::<Vec<U512>>();

    // Calculate amount of liquidity tokens released:
    let lp_emission = U512::from(LP_EMISSION);
    let supply_lp = lp_emission - *lp_amount_before;
    let (relevant_delta_lp_amount, final_lp_amount) = if d2_ > d0 {
        let relevant_lp_amount = (d2_ - d0) * supply_lp / d0;
        assert!(*lp_amount_before > relevant_lp_amount);
        (relevant_lp_amount, *lp_amount_before - relevant_lp_amount)
    } else {
        let relevant_lp_amount = (d0 - d2_) * supply_lp / d0;
        (relevant_lp_amount, *lp_amount_before + relevant_lp_amount)
    };
    // Minimal validity checks from the pool contract:
    let ann = U512::from(ampl_coefficient * n_assets.pow(*n_assets));

    assert_eq!(collected_protocol_fees_final.len() as u32, *n_assets);
    assert!(check_invariant_extremum(
        &n_assets,
        &ann,
        &balances_after_calc,
        &d1,
    ));
    assert!(check_invariant_extremum(
        &n_assets,
        &ann,
        &final_tradable_balances_calc,
        &d2,
    ));
    (
        collected_protocol_fees_final,
        final_lp_amount,
        d1,
        d2,
        relevant_delta_lp_amount,
    )
}

/// Calculates valid transition of the StablePool reserves in the Redeem uniform action.
///
/// # Arguments
///
/// * `reserves_before` - Total reserves of the pool before the action;
/// * `redeemed_lp_amount` - Amount of liquidity tokens to redeem;
/// * `lp_amount_before` - Liquidity reserves of the pool before the action;
/// * `reserves_tokens_decimals` - Decimals of the tradable tokens;
/// * `collected_protocol_fees` - Total collected protocol fees before the action (from pool.datum);
/// * `ampl_coefficient` - Amplification coefficient of the StableSwap invariant (from pool.datum);
/// * `n_assets` - Number of tradable tokens in the pool.
///
/// # Outputs
///
/// * `received_reserves_amounts` - Amount of the pool reserve assets to receive;
/// * `final_balances` - Final reserves of the pool after the action;
/// * `final_lp_amount` - Final amount of the LP tokens in the pool after the action;
/// * `d2` - Invariant value (put into pool.datum).
pub fn redeem_uniform(
    reserves_before: &Vec<U512>,
    redeemed_lp_amount: &U512,
    lp_amount_before: &U512,
    reserves_tokens_decimals: &Vec<U512>,
    collected_protocol_fees: &Vec<U512>,
    ampl_coefficient: &u32,
    n_assets: &u32,
) -> (Vec<U512>, Vec<U512>, U512, U512) {
    assert_eq!(*n_assets, reserves_before.len() as u32);
    assert_eq!(*n_assets, collected_protocol_fees.len() as u32);
    assert_eq!(*n_assets, reserves_tokens_decimals.len() as u32);

    // Calculate balances without collected protocol fees:
    let balances_before_no_fees = reserves_before
        .iter()
        .enumerate()
        .map(|(k, &x)| x - collected_protocol_fees[k])
        .collect::<Vec<U512>>();

    // Calculate amount of the pool reserve assets to receive:
    let lp_emission = U512::from(LP_EMISSION);
    let supply_lp = lp_emission - *lp_amount_before;
    let received_reserves_amounts = balances_before_no_fees
        .iter()
        .map(|&x| (x * *redeemed_lp_amount / supply_lp))
        .collect::<Vec<U512>>();

    // Calculate final liquidity token amount and reserves:
    let final_balances_no_fees = balances_before_no_fees
        .clone()
        .iter()
        .enumerate()
        .map(|(k, &x)| (x - received_reserves_amounts[k]))
        .collect::<Vec<U512>>();

    // Calculate invariant value (balances are converted to the equal precision):
    let precision = U512::from(reserves_tokens_decimals.iter().max().unwrap().0.as_slice()[0]);

    let final_balances_calc = final_balances_no_fees
        .iter()
        .enumerate()
        .map(|(k, &x)| (x * precision / reserves_tokens_decimals[k]))
        .collect::<Vec<U512>>();

    let d2 = calculate_invariant(&final_balances_calc, n_assets, ampl_coefficient);

    // Calculate final balances with fees:
    let final_balances = final_balances_no_fees
        .iter()
        .enumerate()
        .map(|(k, &x)| x + collected_protocol_fees[k])
        .collect::<Vec<U512>>();

    // Calculate final liquidity token amount:
    let final_lp_amount = *lp_amount_before + *redeemed_lp_amount;

    // Minimal validity checks from the pool contract:
    let ann = U512::from(ampl_coefficient * n_assets.pow(*n_assets));
    assert_eq!(received_reserves_amounts.len() as u32, *n_assets);
    assert!(check_invariant_extremum(
        &n_assets,
        &ann,
        &final_balances_calc,
        &d2,
    ));

    (received_reserves_amounts, final_balances, final_lp_amount, d2)
}

/// Calculates valid transition of the StablePool reserves in the Swap action.
///
/// # Arguments
///
/// * `i` - Index of the "base" asset, i.e.index of it's amount in the `reserves_before` vector;
/// * `j` - Index of the "quote" asset, i.e.index of it's amount in the `reserves_before` vector;
/// * `base_amount` - Amount of the "base" asset to be exchanged for the "quote".
/// * `reserves_before` - Total reserves of the pool before the action;
/// * `reserves_tokens_decimals` - Decimals of the tradable tokens;
/// * `collected_protocol_fees` - Total collected protocol fees before the action (from pool.datum);
/// * `swap_fee_num` - Numerator of the swap fee;
/// * `protocol_fee_num` - Numerator of the protocol's fee share;
/// * `ampl_coefficient` - Amplification coefficient of the StableSwap invariant (from pool.datum);
/// * `n_assets` - Number of tradable tokens in the pool.
///
///
/// # Outputs
///
/// * `final_lp_amount` - Final amount of the LP tokens in the pool after the action;
/// * `collected_protocol_fees_final` - Total collected protocol fees after the action (put into pool.datum);
/// * `d1` - Native invariant value (put into pool.datum);
/// * `d2` - Invariant value (put into pool.datum);
/// * `final_reserves` - Total reserves of the pool after the action.
pub fn swap(
    i: &usize,
    j: &usize,
    base_amount: &U512,
    reserves_before: &Vec<U512>,
    collected_protocol_fees: &Vec<U512>,
    reserves_tokens_decimals: &Vec<U512>,
    swap_fee_num: &u32,
    protocol_fee_num: &u32,
    ampl_coefficient: &u32,
    n_assets: &u32,
) -> (Vec<U512>, Vec<U512>, U512, U512, U512) {
    assert_eq!(*n_assets, reserves_before.len() as u32);
    assert_eq!(*n_assets, reserves_tokens_decimals.len() as u32);
    assert_eq!(*n_assets, collected_protocol_fees.len() as u32);

    // Constants commonly used in calculations:
    let unit = U512::from(1);
    let unit_x2 = U512::from(2);
    let nn = U512::from(n_assets.pow(*n_assets));
    let ann = U512::from(*ampl_coefficient) * nn;

    // Calculate initial balances without collected protocol fees:
    let balances_before_no_fees = reserves_before
        .iter()
        .enumerate()
        .map(|(k, &x)| (x - collected_protocol_fees[k]))
        .collect::<Vec<U512>>();

    // Add `base_amount` to the `i`-th reserves balance:
    let mut balances_with_i_no_fees = balances_before_no_fees.clone();
    balances_with_i_no_fees[*i] = balances_with_i_no_fees[*i] + *base_amount;

    // Convert balances to the equal precision for calculations:
    let precision = U512::from(reserves_tokens_decimals.iter().max().unwrap().0.as_slice()[0]);

    let balances_before_no_fees_calc = balances_before_no_fees
        .iter()
        .enumerate()
        .map(|(k, &x)| x * precision / reserves_tokens_decimals[k])
        .collect::<Vec<U512>>();

    let mut balances_with_i_no_fees_calc = balances_with_i_no_fees
        .iter()
        .enumerate()
        .map(|(k, &x)| x * precision / reserves_tokens_decimals[k])
        .collect::<Vec<U512>>();

    // Remove "quote" asset (`j`-th balance) from balances:
    let balances_no_j_calc: Vec<U512> = balances_with_i_no_fees_calc
        .iter()
        .copied()
        .enumerate()
        .filter(|&(k, _)| k != (*j).try_into().unwrap())
        .map(|(_, x)| x)
        .collect();

    // ============== Numerical calculation of the "quote" balance after Swap ==============
    let s = balances_no_j_calc.iter().copied().reduce(|a, b| a + b).unwrap();
    let p = balances_no_j_calc
        .iter()
        .copied()
        .reduce(|a, b| if a != U512::from(0) { a * b } else { b })
        .unwrap();

    let balance_j_initial_calc = balances_before_no_fees_calc[*j].clone();
    let mut balance_j = balance_j_initial_calc.clone();

    // Get current invariant value (must be preserved in the calculations below):
    let d0 = calculate_invariant(&balances_before_no_fees_calc, n_assets, ampl_coefficient);

    let b = s + d0 / ann;
    let c = vec![d0; usize::try_from(*n_assets + 1).unwrap()]
        .iter()
        .copied()
        .reduce(|a, b| a * b)
        .unwrap()
        / nn
        / p
        / ann;

    let mut abs_j_error = d0;
    while abs_j_error > unit {
        let balance_j_previous = balance_j;
        balance_j = (balance_j_previous * balance_j_previous + c) / (unit_x2 * balance_j_previous + b - d0);
        balance_j =
            (balance_j * reserves_tokens_decimals[*j] / precision) * precision / reserves_tokens_decimals[*j];
        abs_j_error = if balance_j >= balance_j_previous {
            balance_j - balance_j_previous
        } else {
            balance_j_previous - balance_j
        }
    }

    // Adjust the calculated final balance of the "quote" asset.
    // Note: value is adjusted at least by 'minimal_j_unit'.
    let minimal_j_unit = precision / reserves_tokens_decimals[*j];
    balances_with_i_no_fees_calc[*j] = balance_j;
    let mut inv_err =
        calculate_invariant_error_from_balances(n_assets, &ann, &balances_with_i_no_fees_calc, &d0);
    balances_with_i_no_fees_calc[*j] = balance_j + minimal_j_unit;
    let inv_err_upper =
        calculate_invariant_error_from_balances(n_assets, &ann, &balances_with_i_no_fees_calc, &d0);
    balances_with_i_no_fees_calc[*j] = balance_j - minimal_j_unit;
    let inv_err_lower =
        calculate_invariant_error_from_balances(n_assets, &ann, &balances_with_i_no_fees_calc, &d0);

    if !((inv_err < inv_err_upper) && (inv_err < inv_err_lower)) {
        let mut inv_err_previous = inv_err + minimal_j_unit;
        while inv_err < inv_err_previous {
            inv_err_previous = inv_err;
            balances_with_i_no_fees_calc[*j] = balances_with_i_no_fees_calc[*j] + minimal_j_unit;
            inv_err =
                calculate_invariant_error_from_balances(n_assets, &ann, &balances_with_i_no_fees_calc, &d0);
        }
        balance_j = balances_with_i_no_fees_calc[*j] - minimal_j_unit;
    }

    let quote_asset_delta = (balance_j_initial_calc - balance_j) * reserves_tokens_decimals[*j] / precision;
    balances_with_i_no_fees_calc[*j] = balance_j;
    // Calculate protocol fees:
    let swap_fee_num_calc = U512::from(*swap_fee_num);
    let protocol_fee_num = U512::from(*protocol_fee_num);
    let denom = U512::from(DENOM);

    let lp_fee = quote_asset_delta * swap_fee_num_calc / denom;

    let protocol_fee_j = quote_asset_delta * protocol_fee_num / denom;

    let total_fees_j = lp_fee + protocol_fee_j;

    // Calculate final protocol fees:
    let mut collected_protocol_fees_final = collected_protocol_fees.clone();
    collected_protocol_fees_final[*j] = collected_protocol_fees_final[*j] + protocol_fee_j;

    // Calculate received "quote" amount:
    let quote_amount_received = quote_asset_delta - total_fees_j;

    // Calculate final balances:
    balances_with_i_no_fees_calc[*j] =
        balances_with_i_no_fees_calc[*j] + lp_fee * precision / reserves_tokens_decimals[*j];

    let final_reserves = balances_with_i_no_fees_calc
        .iter()
        .enumerate()
        .map(|(k, &x)| x * reserves_tokens_decimals[k] / precision + collected_protocol_fees_final[k])
        .collect::<Vec<U512>>();

    // Calculate values of the StableSwap invariant:
    let mut reserves_for_d1_calc = final_reserves
        .iter()
        .enumerate()
        .map(|(k, &x)| (x - collected_protocol_fees_final[k]) * precision / reserves_tokens_decimals[k])
        .collect::<Vec<U512>>();
    reserves_for_d1_calc[*j] = reserves_for_d1_calc[*j] - lp_fee * precision / reserves_tokens_decimals[*j];

    let d1 = calculate_invariant(&reserves_for_d1_calc, n_assets, ampl_coefficient);

    let mut reserves_for_d2_calc = final_reserves
        .iter()
        .enumerate()
        .map(|(k, &x)| (x - collected_protocol_fees[k]) * precision / reserves_tokens_decimals[k])
        .collect::<Vec<U512>>();

    reserves_for_d2_calc[*j] =
        reserves_for_d2_calc[*j] - protocol_fee_j * precision / reserves_tokens_decimals[*j];

    let d2 = calculate_invariant(&reserves_for_d2_calc, n_assets, ampl_coefficient);

    // Minimal validity checks from the pool contract:
    assert_eq!(final_reserves[*i] - reserves_before[*i], *base_amount);
    assert!(reserves_before[*j] - final_reserves[*j] <= quote_amount_received + unit);
    assert_eq!(collected_protocol_fees_final.len() as u32, *n_assets);
    assert!(check_invariant_extremum_for_asset(
        &n_assets,
        &ann,
        &reserves_for_d1_calc,
        &d1,
        &minimal_j_unit,
        j,
    ));
    assert!(check_invariant_extremum_for_asset(
        &n_assets,
        &ann,
        &reserves_for_d2_calc,
        &d2,
        &minimal_j_unit,
        j,
    ));

    let abs_inv_diff = if d1 > d0 { d1 - d0 } else { d0 - d1 };
    assert!(abs_inv_diff <= unit_x2 * precision / reserves_tokens_decimals[*j]);

    (
        final_reserves,
        collected_protocol_fees_final,
        d1,
        d2,
        quote_amount_received,
    )
}

#[cfg(test)]
mod test {
    use primitive_types::U512;
    use rand::distributions::Standard;
    use rand::rngs::StdRng;
    use rand::seq::SliceRandom;
    use rand::{Rng, SeedableRng};

    use crate::stable_swap_amm_actions::{
        liquidity_action, redeem_uniform, swap, DENOM, LP_EMISSION,
    };
    use crate::stable_swap_invariant::calculate_invariant;

    /// Prepare random stable3pool parameters and balances.
    fn prepare_random_s3pool_state(
        mut rng: StdRng,
        n: u32,
    ) -> (u32, u32, u32, Vec<U512>, Vec<U512>, Vec<U512>, U512) {
        // Random parameters:
        let a: u32 = rng.gen_range(100..2000);
        let swap_fee_num: u32 = rng.gen_range(1_000..5_000);
        let protocol_fee_num: u32 = rng.gen_range(50..5_00);

        // Tradable tokens decimals:
        let mut reserves_tokens_decimals = Vec::new();
        for _ in 0..n {
            let exp: u32 = rng.gen_range(3..9);
            reserves_tokens_decimals.push(U512::from(10_u64.pow(exp)))
        }
        // Initial tradable reserves:
        let tradable_reserves_before: Vec<U512> = vec![U512::from(u64::MAX); n as usize]
            .iter()
            .enumerate()
            .map(|(k, &x)| x * reserves_tokens_decimals[k])
            .collect::<Vec<U512>>();

        // Collected protocol fees:
        let collected_protocol_fees: Vec<U512> = rand::thread_rng()
            .sample_iter::<u32, Standard>(Standard)
            .take(n.try_into().unwrap())
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
            .map(|(k, x)| U512::from(x) * reserves_tokens_decimals[k] + U512::from(x))
            .collect::<Vec<U512>>();

        // Total initial reserves (tradable reserves + collected fees):
        let reserves_before = tradable_reserves_before
            .iter()
            .enumerate()
            .map(|(k, &x)| x + collected_protocol_fees[k])
            .collect::<Vec<U512>>();

        // Precision of calculations:
        let precision = reserves_tokens_decimals.iter().max().unwrap().0.as_slice()[0];

        // Initial supply lp (approximation for tests):
        let reserves_before_calc = reserves_before
            .iter()
            .enumerate()
            .map(|(k, x)| U512::from(x) * precision / reserves_tokens_decimals[k])
            .collect::<Vec<U512>>();

        let initial_supply_lp = calculate_invariant(&reserves_before_calc, &n, &a);

        (
            a,
            swap_fee_num,
            protocol_fee_num,
            reserves_before,
            collected_protocol_fees,
            reserves_tokens_decimals,
            initial_supply_lp,
        )
    }

    #[test]
    /// Test arbitrary deposit (same logic for uniform deposit).
    fn deposit_test() {
        let n_assets_set: Vec<u32> = vec![2, 3, 4];
        let rng = StdRng::seed_from_u64(42);
        for _ in 0..100000 {
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let (
                a,
                swap_fee_num,
                protocol_fee_num,
                reserves_before,
                collected_protocol_fees,
                reserves_tokens_decimals,
                initial_supply_lp,
            ) = prepare_random_s3pool_state(rng.clone(), n);

            let deposited_reserves: Vec<U512> = rand::thread_rng()
                .sample_iter::<u32, Standard>(Standard)
                .take(n.try_into().unwrap())
                .collect::<Vec<_>>()
                .into_iter()
                .enumerate()
                .map(|(k, x)| U512::from(x) * reserves_tokens_decimals[k] + U512::from(x))
                .collect::<Vec<U512>>();

            let reserves_after = reserves_before
                .iter()
                .enumerate()
                .map(|(k, &x)| x + deposited_reserves[k])
                .collect::<Vec<U512>>();
            let lp_amount_before =
                U512::from(LP_EMISSION) - initial_supply_lp;

            liquidity_action(
                &reserves_before,
                &reserves_after,
                &lp_amount_before,
                &reserves_tokens_decimals,
                &collected_protocol_fees,
                &swap_fee_num,
                &protocol_fee_num,
                &a,
                &n,
            );
        }
    }

    #[test]
    /// Test arbitrary redeem (required LP token amount for desired received tokens amounts).
    fn redeem_arbitrary_test() {
        let n_assets_set: Vec<u32> = vec![2, 3, 4];
        let rng = StdRng::seed_from_u64(42);
        for _ in 0..100000 {
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let (
                a,
                swap_fee_num,
                protocol_fee_num,
                reserves_before,
                collected_protocol_fees,
                reserves_tokens_decimals,
                initial_supply_lp,
            ) = prepare_random_s3pool_state(rng.clone(), n);

            let desired_reserves: Vec<U512> = rand::thread_rng()
                .sample_iter::<u32, Standard>(Standard)
                .take(n.try_into().unwrap())
                .collect::<Vec<_>>()
                .into_iter()
                .enumerate()
                .map(|(k, x)| U512::from(x) * reserves_tokens_decimals[k] + U512::from(x))
                .collect::<Vec<U512>>();

            let reserves_after = reserves_before
                .iter()
                .enumerate()
                .map(|(k, &x)| x - desired_reserves[k])
                .collect::<Vec<U512>>();

            let lp_amount_before =
                U512::from(LP_EMISSION) - initial_supply_lp;

            liquidity_action(
                &reserves_before,
                &reserves_after,
                &lp_amount_before,
                &reserves_tokens_decimals,
                &collected_protocol_fees,
                &swap_fee_num,
                &protocol_fee_num,
                &a,
                &n,
            );
        }
    }

    #[test]
    /// Test uniform redeem (received tokens amounts for redeemed LP token amount).
    fn redeem_uniform_test() {
        let n_assets_set: Vec<u32> = vec![2, 3, 4];
        let rng = StdRng::seed_from_u64(42);
        for _ in 0..10000 {
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let (
                a,
                _,
                _,
                reserves_before,
                collected_protocol_fees,
                reserves_tokens_decimals,
                initial_supply_lp,
            ) = prepare_random_s3pool_state(rng.clone(), n);

            let lp_amount_before =
                U512::from(LP_EMISSION) - initial_supply_lp;
            redeem_uniform(
                &reserves_before,
                &(initial_supply_lp - U512::from(1)),
                &lp_amount_before,
                &reserves_tokens_decimals,
                &collected_protocol_fees,
                &a,
                &n,
            );
        }
    }

    #[test]
    /// Test swap (i -> j), arbitrary amount.
    fn swap_test() {
        let n_assets_set: Vec<u32> = vec![3, 4];
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..10000 {
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let (
                a,
                swap_fee_num,
                protocol_fee_num,
                reserves_before,
                collected_protocol_fees,
                reserves_tokens_decimals,
                _,
            ) = prepare_random_s3pool_state(rng.clone(), n);

            let inds = rand::seq::index::sample(&mut rng, n as usize, 2).into_vec();
            let i = *inds.get(0).unwrap();
            let j = *inds.get(1).unwrap();

            let base_amount = U512::from(rng.gen_range(DENOM as u32..u32::MAX)) * reserves_tokens_decimals[i];

            swap(
                &i,
                &j,
                &base_amount,
                &reserves_before,
                &collected_protocol_fees,
                &reserves_tokens_decimals,
                &swap_fee_num,
                &protocol_fee_num,
                &a,
                &n,
            );
        }
    }

    #[test]
    /// Test full protocol flow.
    fn full_flow_test() {
        let n_assets_set: Vec<u32> = vec![3, 4];
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..100 {
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let (a, swap_fee_num, protocol_fee_num, _, _, reserves_tokens_decimals, _) =
                prepare_random_s3pool_state(rng.clone(), n);

            let collected_protocol_fees = vec![U512::from(0); n as usize];
            // First deposit:
            let reserves_before_usd = vec![U512::from(1_000_000u64); n as usize];
            let reserves_before = reserves_before_usd
                .iter()
                .enumerate()
                .map(|(k, x)| U512::from(x) * reserves_tokens_decimals[k])
                .collect::<Vec<U512>>();

            // Precision of calculations:
            let precision = reserves_tokens_decimals.iter().max().unwrap().0.as_slice()[0];

            // Initial supply lp (approximation for tests):
            let reserves_before_calc = reserves_before
                .iter()
                .enumerate()
                .map(|(k, x)| U512::from(x) * precision / reserves_tokens_decimals[k])
                .collect::<Vec<U512>>();

            let deposited_reserves0_usd = vec![U512::from(1000); n as usize];
            let deposited_reserves0 = deposited_reserves0_usd
                .iter()
                .enumerate()
                .map(|(k, x)| U512::from(x) * reserves_tokens_decimals[k])
                .collect::<Vec<U512>>();

            let reserves_after0 = reserves_before
                .iter()
                .enumerate()
                .map(|(k, &x)| x + deposited_reserves0[k])
                .collect::<Vec<U512>>();

            let initial_supply_lp = calculate_invariant(&reserves_before_calc, &n, &a);
            let lp_amount_before0 =
                U512::from(LP_EMISSION) - initial_supply_lp;

            let (collected_protocol_fees0, lp_amount_before1, _, _, received_lp0) = liquidity_action(
                &reserves_before,
                &reserves_after0,
                &lp_amount_before0,
                &reserves_tokens_decimals,
                &collected_protocol_fees,
                &swap_fee_num,
                &protocol_fee_num,
                &a,
                &n,
            );

            // Second deposit:
            let deposited_reserves1_usd = vec![U512::from(2) * U512::from(1000); n as usize];
            let deposited_reserves1 = deposited_reserves1_usd
                .iter()
                .enumerate()
                .map(|(k, x)| U512::from(x) * reserves_tokens_decimals[k])
                .collect::<Vec<U512>>();
            let reserves_after1 = reserves_after0
                .iter()
                .enumerate()
                .map(|(k, &x)| x + deposited_reserves1[k])
                .collect::<Vec<U512>>();

            let (collected_protocol_fees1, lp_amount_before2, _, _, received_lp1) = liquidity_action(
                &reserves_after0,
                &reserves_after1,
                &lp_amount_before1,
                &reserves_tokens_decimals,
                &collected_protocol_fees0,
                &swap_fee_num,
                &protocol_fee_num,
                &a,
                &n,
            );

            // Series of swaps:
            let mut reserves_after_swaps = reserves_after1.clone();
            let mut collected_protocol_fees_after_swaps = collected_protocol_fees1.clone();
            let mut total_volume = U512::from(0);
            for _ in 0..100 {
                let inds = rand::seq::index::sample(&mut rng, n as usize, 2).into_vec();

                let i = *inds.get(0).unwrap();
                let j = *inds.get(1).unwrap();
                let base_amount =
                    U512::from(rng.gen_range(DENOM as u32..u16::MAX as u32)) * reserves_tokens_decimals[i];

                // let base_amount =
                //     U512::from(rng.gen_range(precision..u32::MAX as u64)) * reserves_tokens_decimals[i];

                (reserves_after_swaps, collected_protocol_fees_after_swaps, _, _, _) = swap(
                    &i,
                    &j,
                    &base_amount,
                    &reserves_after_swaps,
                    &collected_protocol_fees_after_swaps,
                    &reserves_tokens_decimals,
                    &swap_fee_num,
                    &protocol_fee_num,
                    &a,
                    &n,
                );
                reserves_after_swaps = reserves_after_swaps.clone();
                collected_protocol_fees_after_swaps = collected_protocol_fees_after_swaps.clone();
                total_volume = total_volume + base_amount / reserves_tokens_decimals[i];
            }
            // First redeem:
            let (received_reserves_amounts0, _, _, _) = redeem_uniform(
                &reserves_after_swaps,
                &received_lp0,
                &lp_amount_before2,
                &reserves_tokens_decimals,
                &collected_protocol_fees_after_swaps,
                &a,
                &n,
            );

            let reserves_after_first_redeem = reserves_after_swaps
                .iter()
                .enumerate()
                .map(|(k, &x)| x - received_reserves_amounts0[k])
                .collect::<Vec<U512>>();
            let (_, _, _, _, relevant_delta_lp_amount) = liquidity_action(
                &reserves_after_swaps,
                &reserves_after_first_redeem,
                &lp_amount_before2,
                &reserves_tokens_decimals,
                &collected_protocol_fees_after_swaps,
                &swap_fee_num,
                &protocol_fee_num,
                &a,
                &n,
            );
            assert!(received_lp0 - relevant_delta_lp_amount <= U512::from(precision));
            // Second redeem:
            let (received_reserves_amounts1, _, _, _) = redeem_uniform(
                &reserves_after_swaps,
                &(received_lp1 - U512::from(1)),
                &lp_amount_before2,
                &reserves_tokens_decimals,
                &collected_protocol_fees_after_swaps,
                &a,
                &n,
            );

            let usd_deposited0: U512 = deposited_reserves0
                .iter()
                .enumerate()
                .map(|(k, &x)| x / reserves_tokens_decimals[k])
                .collect::<Vec<U512>>()
                .iter()
                .copied()
                .reduce(|a, b| a + b)
                .unwrap();

            let usd_received0: U512 = received_reserves_amounts0
                .iter()
                .enumerate()
                .map(|(k, &x)| x / reserves_tokens_decimals[k])
                .collect::<Vec<U512>>()
                .iter()
                .copied()
                .reduce(|a, b| a + b)
                .unwrap();

            let usd_deposited1: U512 = deposited_reserves1
                .iter()
                .enumerate()
                .map(|(k, &x)| x / reserves_tokens_decimals[k])
                .collect::<Vec<U512>>()
                .iter()
                .copied()
                .reduce(|a, b| a + b)
                .unwrap();

            let usd_received1: U512 = received_reserves_amounts1
                .iter()
                .enumerate()
                .map(|(k, &x)| x / reserves_tokens_decimals[k])
                .collect::<Vec<U512>>()
                .iter()
                .copied()
                .reduce(|a, b| a + b)
                .unwrap();

            let usd_before1: U512 = reserves_after1
                .iter()
                .enumerate()
                .map(|(k, &x)| x / reserves_tokens_decimals[k])
                .collect::<Vec<U512>>()
                .iter()
                .copied()
                .reduce(|a, b| a + b)
                .unwrap();

            let usd_after1: U512 = reserves_after_swaps
                .iter()
                .enumerate()
                .map(|(k, &x)| x / reserves_tokens_decimals[k])
                .collect::<Vec<U512>>()
                .iter()
                .copied()
                .reduce(|a, b| a + b)
                .unwrap();

            let collected_protocol_fees_usd: U512 = collected_protocol_fees_after_swaps
                .iter()
                .enumerate()
                .map(|(k, &x)| x / reserves_tokens_decimals[k])
                .collect::<Vec<U512>>()
                .iter()
                .copied()
                .reduce(|a, b| a + b)
                .unwrap();
            // Checks:
            let usd_profit0 = usd_received0 - usd_deposited0;
            let usd_profit1 = usd_received1 - usd_deposited1;
            let total_usd_profit = usd_after1 - usd_before1;
            let lp_usd_profit = usd_profit0 + usd_profit1;
            let lp_usd_profit_theoretical = total_volume * (received_lp0 + received_lp1)
                / (initial_supply_lp + received_lp0 + received_lp1) * U512::from(swap_fee_num)
                * U512::from(90)
                / U512::from(DENOM) / U512::from(100);
            let total_protocol_usd_profit_theoretical = total_volume
                * U512::from(protocol_fee_num)
                / U512::from(DENOM);
            let total_lp_usd_profit_theoretical = total_volume
                * U512::from(swap_fee_num)
                / U512::from(DENOM);
            assert_eq!(
                (lp_usd_profit * precision / total_usd_profit) / precision,
                ((received_lp0 + received_lp1) * precision
                    / (initial_supply_lp + received_lp0 + received_lp1))
                    / precision
            );
            assert!(
                collected_protocol_fees_usd
                    >= total_volume
                    * U512::from(protocol_fee_num)
                    * U512::from(80)
                    / U512::from(DENOM) / U512::from(100)
            );
            assert!(
                lp_usd_profit
                    >= lp_usd_profit_theoretical * U512::from(80)
                    / U512::from(100)
            );

            assert!(
                total_usd_profit
                    <= (total_protocol_usd_profit_theoretical + total_lp_usd_profit_theoretical) * U512::from(110)
                    / U512::from(100)
            );
            assert_eq!(
                (usd_received1 * precision / usd_received0) / precision,
                (received_lp1 * precision / received_lp0) / precision
            );
        }
    }
}
