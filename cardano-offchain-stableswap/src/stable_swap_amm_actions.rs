use primitive_types::U512;

use crate::stable_swap_invariant::{
    calculate_invariant, calculate_invariant_error_from_balances, check_invariant_extremum,
};

pub(crate) const LP_EMISSION: u64 = 9223372036854775807;
pub(crate) const LP_NUM_DECIMALS: u32 = 9;
pub(crate) const DENOM: u64 = 10_000;

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
/// * `protocol_share_num` - Numerator of the protocol's fee share;
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
    protocol_share_num: &u32,
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
    let fee_num = U512::from((swap_fee_num * n_assets) / (4 * (n_assets - 1)));

    let protocol_fees_native = difference
        .iter()
        .map(|&x| (fee_num * x * U512::from(*protocol_share_num)) / (denom * denom))
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

    // Calculate final value of the invariant:
    let d2 = calculate_invariant(&final_tradable_balances_calc, n_assets, ampl_coefficient);

    // Calculate final collected protocol fees:
    let collected_protocol_fees_final = collected_protocol_fees
        .iter()
        .enumerate()
        .map(|(k, &x)| x + protocol_fees_native[k])
        .collect::<Vec<U512>>();

    // Calculate amount of liquidity tokens released:
    let lp_emission = U512::from(LP_EMISSION) * U512::from(10_u64.pow(LP_NUM_DECIMALS));
    let supply_lp = lp_emission - *lp_amount_before;
    let (relevant_delta_lp_amount, final_lp_amount) = if d2 > d0 {
        let relevant_lp_amount = (d2 - d0) * supply_lp / d0;
        assert!(*lp_amount_before > relevant_lp_amount);
        (relevant_lp_amount, *lp_amount_before - relevant_lp_amount)
    } else {
        let relevant_lp_amount = (d0 - d2) * supply_lp / d0;
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
/// * `final_balances` - Final reserves of the pool after the action;;
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
        .map(|(k, &x)| (x - collected_protocol_fees[k]))
        .collect::<Vec<U512>>();

    // Calculate amount of the pool reserve assets to receive:
    let lp_emission = U512::from(LP_EMISSION) * U512::from(10_u64.pow(LP_NUM_DECIMALS));
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

    // Minimal validity checks from the pool contract:
    let ann = U512::from(ampl_coefficient * n_assets.pow(*n_assets));
    assert_eq!(received_reserves_amounts.len() as u32, *n_assets);
    assert!(check_invariant_extremum(
        &n_assets,
        &ann,
        &final_balances_calc,
        &d2,
    ));

    // Calculate final balances with fees:
    let final_balances = final_balances_no_fees
        .iter()
        .enumerate()
        .map(|(k, &x)| x + collected_protocol_fees[k])
        .collect::<Vec<U512>>();

    // Calculate final liquidity token amount:
    let final_lp_amount = *lp_amount_before + *redeemed_lp_amount;

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
/// * `protocol_share_num` - Numerator of the protocol's fee share;
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
    protocol_share_num: &u32,
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

    // Adjust the calculated final balance of the "quote" asset:
    balances_with_i_no_fees_calc[*j] = balance_j;
    let mut inv_err =
        calculate_invariant_error_from_balances(n_assets, &ann, &balances_with_i_no_fees_calc, &d0);
    balances_with_i_no_fees_calc[*j] = balance_j + unit;
    let inv_err_upper =
        calculate_invariant_error_from_balances(n_assets, &ann, &balances_with_i_no_fees_calc, &d0);
    balances_with_i_no_fees_calc[*j] = balance_j - unit;
    let inv_err_lower =
        calculate_invariant_error_from_balances(n_assets, &ann, &balances_with_i_no_fees_calc, &d0);

    if !((inv_err < inv_err_upper) && (inv_err < inv_err_lower)) {
        let mut inv_err_previous = inv_err + unit;
        while inv_err < inv_err_previous {
            inv_err_previous = inv_err;
            balances_with_i_no_fees_calc[*j] = balances_with_i_no_fees_calc[*j] + unit;
            inv_err =
                calculate_invariant_error_from_balances(n_assets, &ann, &balances_with_i_no_fees_calc, &d0);
        }
        balance_j = balances_with_i_no_fees_calc[*j] - unit * precision / reserves_tokens_decimals[*j];
    }

    let quote_asset_delta_calc = balance_j_initial_calc - balance_j;
    balances_with_i_no_fees_calc[*j] = balance_j_initial_calc - quote_asset_delta_calc;

    // Calculate output (d1 aka native) value of the StableSwap invariant:
    let balances_with_no_fees_calc = balances_with_i_no_fees_calc.clone();
    let d1 = calculate_invariant(&balances_with_no_fees_calc, n_assets, ampl_coefficient);

    // Calculate protocol fees:
    let swap_fee_num_calc = U512::from(*swap_fee_num);
    let protocol_share_num = U512::from(*protocol_share_num);
    let denom = U512::from(DENOM);

    let total_fees_j = ((quote_asset_delta_calc * swap_fee_num_calc / denom) * reserves_tokens_decimals[*j]
        / precision)
        * precision
        / reserves_tokens_decimals[*j];
    let protocol_fees_j = (quote_asset_delta_calc * swap_fee_num_calc * protocol_share_num / (denom * denom)
        * reserves_tokens_decimals[*j]
        / precision)
        * precision
        / reserves_tokens_decimals[*j];
    let mut collected_protocol_fees_final = collected_protocol_fees.clone();
    collected_protocol_fees_final[*j] =
        collected_protocol_fees_final[*j] + protocol_fees_j * reserves_tokens_decimals[*j] / precision;

    // Calculate received "quote" amount:
    let quote_amount_received =
        (quote_asset_delta_calc - total_fees_j) * reserves_tokens_decimals[*j] / precision;

    // Calculate values of the StableSwap invariant:
    balances_with_i_no_fees_calc[*j] = balances_with_i_no_fees_calc[*j] + (total_fees_j - protocol_fees_j);
    let final_balances_calc = balances_with_i_no_fees_calc.clone();
    let d2 = calculate_invariant(&final_balances_calc, n_assets, ampl_coefficient);

    // Calculate final balances:
    let final_reserves = balances_with_i_no_fees_calc
        .iter()
        .enumerate()
        .map(|(k, &x)| x * reserves_tokens_decimals[k] / precision + collected_protocol_fees_final[k])
        .collect::<Vec<U512>>();

    // Minimal validity checks from the pool contract:
    assert_eq!(final_reserves[*i] - reserves_before[*i], *base_amount);
    assert!(reserves_before[*j] - final_reserves[*j] <= quote_amount_received + unit_x2);
    assert_eq!(collected_protocol_fees_final.len() as u32, *n_assets);
    assert!(check_invariant_extremum(
        &n_assets,
        &ann,
        &balances_with_no_fees_calc,
        &d1,
    ));
    assert!(check_invariant_extremum(
        &n_assets,
        &ann,
        &final_balances_calc,
        &d2,
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
