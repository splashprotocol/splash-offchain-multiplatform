use crate::crypto_pool_data::{PoolConfig, PoolReserves, PoolStateData};
use crate::curve_crypto_invariant::{calculate_crypto_exchange, calculate_crypto_invariant};
use crate::fees_utils::{calculate_lp_token_fee, calculate_new_fee_num};
use crate::math_utils::nonzero_prod;
use crate::repegging_utils::tweak_price;
use primitive_types::U512;

pub(crate) const LP_EMISSION: u128 = 340282366920938463463374607431768211455;
pub(crate) const LP_NUM_DECIMALS: u32 = 9;
pub(crate) const FEE_DENOM: u64 = 10_000;
pub(crate) const GAMMA_DENOM: u64 = 1_000_000;
pub(crate) const FEE_GAMMA_DENOM: u64 = 10_000;
pub(crate) const PRECISION: u64 = 1_000_000_000;

/// Calculates valid transition of the StablePool reserves in the arbitrary Deposit/Redeem actions.
///
/// # Arguments
/// * `reserves_before` - Total reserves of the pool before the action;
/// * `tradable_reserves_after` - Total tradable reserves of the pool after the action;
/// * `config` - Configuration of the pool;
/// * `state_before` - State of the pool before the action;
///
/// # Outputs
/// * `reserves_after` - Total reserves of the pool after the action;
/// * `state_after` - State of the pool after the action.
pub fn liquidity_action(
    reserves_before: &PoolReserves,
    tradable_reserves_after: &Vec<U512>,
    config: &PoolConfig,
    state_before: &PoolStateData,
) -> (PoolReserves, PoolStateData) {
    // Constants for calculations:
    let unit = U512::from(1);
    let unit_x4 = U512::from(4);

    let precision = U512::from(PRECISION);
    let n = config.n_tradable_assets;
    let a = config.ampl_coeff;

    let fee_gamma_num = config.fee_gamma_num;
    let fee_mid_num = config.fee_mid_num;
    let fee_out_num = config.fee_out_num;

    let gamma_num = config.gamma_num;
    let gamma_denom = U512::from(GAMMA_DENOM);
    let fee_denom = U512::from(FEE_DENOM);
    let fee_gamma_denom = U512::from(FEE_GAMMA_DENOM);

    // State before:
    let x_cp_profit_before = state_before.xcp_profit.clone();
    let x_cp_profit_real_before = state_before.xcp_profit_real.clone();
    let price_scale_vec = state_before.price_scale.clone();

    // Reserves:
    let tradable_balances_before = reserves_before.tradable.clone();
    let lp_balance_before = reserves_before.lp_tokens.clone();

    assert_eq!(n, (tradable_balances_before.len() as u32).into());
    assert_eq!(n, (tradable_reserves_after.len() as u32).into());
    assert_eq!(n, (price_scale_vec.len() as u32).into());

    // Convert all to the common price scale:
    let tradable_balances_before_calc = tradable_balances_before
        .iter()
        .enumerate()
        .map(|(k, &x)| x * precision / price_scale_vec[k])
        .collect::<Vec<U512>>();

    let tradable_balances_after_calc = tradable_reserves_after
        .iter()
        .enumerate()
        .map(|(k, &x)| x * precision / price_scale_vec[k])
        .collect::<Vec<U512>>();

    // Calculate initial (d0) and output (d1 aka native) values of the CurveCrypto invariant:
    let d0 = calculate_crypto_invariant(
        &tradable_balances_before_calc,
        &n.as_u32(),
        &a,
        &gamma_num,
        &gamma_denom,
    );
    let d1 = calculate_crypto_invariant(
        &tradable_balances_after_calc,
        &n.as_u32(),
        &a,
        &gamma_num,
        &gamma_denom,
    );

    // Calculate amount of LP tokens:
    let lp_supply_before = U512::from(LP_EMISSION) - lp_balance_before;

    let lp_supply_after = lp_supply_before * d1 / d0;
    let lp_delta = if lp_supply_after > lp_supply_before {
        lp_supply_after - lp_supply_before
    } else {
        lp_supply_before - lp_supply_after
    };

    // Calculate fees:
    let s = tradable_balances_after_calc
        .iter()
        .copied()
        .reduce(|x, y| x + y)
        .unwrap();
    let p = nonzero_prod(&tradable_balances_after_calc);

    let fee_num_ = calculate_new_fee_num(
        &n.as_u32(),
        &s,
        &p,
        &fee_gamma_num,
        &fee_mid_num,
        &fee_out_num,
        &fee_gamma_denom,
    );
    let fee_num = fee_num_ * n / (unit_x4 * n) - unit;

    let lp_fee_num = calculate_lp_token_fee(&fee_num, &tradable_balances_after_calc);
    let lp_fees = lp_delta * lp_fee_num / fee_denom;

    let lp_rec = lp_delta - lp_fees;

    // Calculate profits and new price vector:
    let (new_price_vec, x_cp_profit_new, virtual_price_new) = tweak_price(
        &(n.as_u32()),
        &tradable_balances_after_calc,
        &lp_supply_after,
        &price_scale_vec,
        &d1,
        &a,
        &gamma_num,
        &gamma_denom,
        &x_cp_profit_before,
        &x_cp_profit_real_before,
    );
    (
        PoolReserves {
            tradable: (*tradable_reserves_after.clone()).to_vec(),
            lp_tokens: if lp_supply_after > lp_supply_before {
                lp_balance_before - lp_rec
            } else {
                lp_balance_before + lp_rec
            },
        },
        PoolStateData {
            price_scale: new_price_vec,
            xcp_profit: x_cp_profit_new,
            xcp_profit_real: virtual_price_new,
        },
    )
}

/// Calculates valid transition of the StablePool reserves in the swap action.
///
/// # Arguments
/// * `i` - index of base tradable asset;
/// * `j` - index of quote tradable asset;
/// * `base_amount` - Amount of the `i`-th tradable asset to exchange;
/// * `reserves_before` - Total reserves of the pool before the action;
/// * `config` - Configuration of the pool;
/// * `state_before` - State of the pool before the action;
///
/// # Outputs
/// * `reserves_after` - Total reserves of the pool after the action;
/// * `state_after` - State of the pool after the action.
pub fn swap(
    i: &usize,
    j: &usize,
    base_amount: &U512,
    reserves_before: &PoolReserves,
    config: &PoolConfig,
    state_before: &PoolStateData,
) -> (PoolReserves, PoolStateData) {
    // Constants for calculations:
    let precision = U512::from(PRECISION);
    let n = config.n_tradable_assets;
    let a = config.ampl_coeff;

    let fee_gamma_num = config.fee_gamma_num;
    let fee_mid_num = config.fee_mid_num;
    let fee_out_num = config.fee_out_num;

    let gamma_num = config.gamma_num;
    let gamma_denom = U512::from(GAMMA_DENOM);
    let fee_denom = U512::from(FEE_DENOM);
    let fee_gamma_denom = U512::from(FEE_GAMMA_DENOM);
    // State before:
    let x_cp_profit_before = state_before.xcp_profit.clone();
    let x_cp_profit_real_before = state_before.xcp_profit_real.clone();
    let price_scale_vec = state_before.price_scale.clone();

    // Reserves:
    let tradable_balances_before = reserves_before.tradable.clone();
    let lp_balance_before = reserves_before.lp_tokens.clone();

    assert_eq!(n, (tradable_balances_before.len() as u32).into());
    assert_eq!(n, (price_scale_vec.len() as u32).into());

    // Convert balances to the equal precision for calculations:
    let tradable_balances_before_calc = tradable_balances_before
        .iter()
        .enumerate()
        .map(|(k, &x)| x * precision / price_scale_vec[k])
        .collect::<Vec<U512>>();

    let base_amount_calc = base_amount * precision / price_scale_vec[*i];

    // Calculate quote amount:
    let quote_amount_final = calculate_crypto_exchange(
        i,
        j,
        &base_amount_calc,
        &tradable_balances_before_calc,
        &(n.as_u32()),
        &a,
        &gamma_num,
        &gamma_denom,
    );

    let mut tradable_balances_after_no_fees_calc = tradable_balances_before_calc.clone();
    tradable_balances_after_no_fees_calc[*j] = quote_amount_final;
    // Calculate protocol fees:
    let s = tradable_balances_after_no_fees_calc
        .iter()
        .copied()
        .reduce(|x, y| x + y)
        .unwrap();

    let p = nonzero_prod(&tradable_balances_after_no_fees_calc);

    let fee_num = calculate_new_fee_num(
        &n.as_u32(),
        &s,
        &p,
        &fee_gamma_num,
        &fee_mid_num,
        &fee_out_num,
        &fee_gamma_denom,
    );

    let quote_amount_final_real_units = quote_amount_final * price_scale_vec[*j] / precision;
    let quote_asset_delta = tradable_balances_before[*j] - quote_amount_final_real_units;

    let total_fees_j = quote_asset_delta * fee_num / fee_denom;

    // Calculate received "quote" amount:
    let quote_amount_received = quote_asset_delta - total_fees_j;

    // Calculate final balances:
    let mut tradable_balances_final = tradable_balances_before.clone();
    tradable_balances_final[*i] += *base_amount;
    tradable_balances_final[*j] -= quote_amount_received;

    let tradable_balances_final_calc = tradable_balances_final
        .iter()
        .clone()
        .enumerate()
        .map(|(k, &x)| x * precision / price_scale_vec[k])
        .collect::<Vec<U512>>();

    // Calculate profits and new price vector:
    let d1 = calculate_crypto_invariant(
        &tradable_balances_final_calc,
        &n.as_u32(),
        &a,
        &gamma_num,
        &gamma_denom,
    );
    let (new_price_vec, x_cp_profit_new, virtual_price_new) = tweak_price(
        &(n.as_u32()),
        &tradable_balances_final_calc,
        &lp_balance_before,
        &price_scale_vec,
        &d1,
        &a,
        &gamma_num,
        &gamma_denom,
        &x_cp_profit_before,
        &x_cp_profit_real_before,
    );
    (
        PoolReserves {
            tradable: tradable_balances_final,
            lp_tokens: lp_balance_before,
        },
        PoolStateData {
            price_scale: new_price_vec,
            xcp_profit: x_cp_profit_new,
            xcp_profit_real: virtual_price_new,
        },
    )
}
#[cfg(test)]
mod test {
    use crate::generators::{pool_config_gen, pool_reserves_gen, random_deposit, random_redeem, random_swap};
    use primitive_types::U512;

    #[test]
    fn imbalanced_deposit_test() {
        for _ in 0..10000 {
            // Generate random config:
            let pool_conf = pool_config_gen();

            // Generate initial reserves:
            let (pool_reserves_before, pool_state_before) = pool_reserves_gen(&pool_conf, &true);

            // Random deposit:
            let (pool_reserves_after, pool_state_after, deposited_amounts, rec_lp) =
                random_deposit(&pool_conf, &pool_reserves_before, &pool_state_before, &false);
        }
    }
    #[test]
    fn uniform_deposit_test() {
        for _ in 0..10000 {
            // Generate random config:
            let pool_conf = pool_config_gen();

            // Generate initial reserves:
            let (pool_reserves_before, pool_state_before) = pool_reserves_gen(&pool_conf, &true);

            // Random deposit:
            let (pool_reserves_after, pool_state_after, deposited_amounts, rec_lp) =
                random_deposit(&pool_conf, &pool_reserves_before, &pool_state_before, &true);
        }
    }
    #[test]
    fn imbalanced_redeem_test() {
        for _ in 0..10000 {
            // Generate random config:
            let pool_conf = pool_config_gen();

            // Generate initial reserves:
            let (pool_reserves_before, pool_state_before) = pool_reserves_gen(&pool_conf, &true);

            // Random redeem:
            let (pool_reserves_after, pool_state_after, redeemed_amounts, req_lp) =
                random_redeem(&pool_conf, &pool_reserves_before, &pool_state_before, &false);
        }
    }
    #[test]
    fn uniform_redeem_test() {
        for _ in 0..10000 {
            // Generate random config:
            let pool_conf = pool_config_gen();

            // Generate initial reserves:
            let (pool_reserves_before, pool_state_before) = pool_reserves_gen(&pool_conf, &true);

            let redeemed_amounts = vec![U512::from(100), U512::from(100), U512::from(1_000)];

            // Random redeem:
            let (pool_reserves_after, pool_state_after, redeemed_amounts, req_lp) =
                random_redeem(&pool_conf, &pool_reserves_before, &pool_state_before, &true);
        }
    }
    #[test]
    fn swap_test() {
        for _ in 0..10 {
            // Generate random config:
            let pool_conf = pool_config_gen();

            // Generate initial reserves:
            let (pool_reserves_init, pool_state_init) = pool_reserves_gen(&pool_conf, &true);

            // Random swap:
            let (pool_reserves_after0, pool_state_after0, quote_rec) =
                random_swap(&pool_conf, &pool_reserves_init, &pool_state_init);
        }
    }
    #[test]
    fn full_flow_test() {
        for _ in 0..1 {
            // Generate random config:
            let pool_conf = pool_config_gen();

            // Generate initial reserves:
            let (pool_reserves_init, pool_state_init) = pool_reserves_gen(&pool_conf, &true);

            // First LP deposit:
            let (pool_reserves_after0, pool_state_after0, deposited_amounts0, rec_lp0) =
                random_deposit(&pool_conf, &pool_reserves_init, &pool_state_init, &false);

            // Second LP deposit:
            let (pool_reserves_after1, pool_state_after1, deposited_amounts1, rec_lp1) =
                random_deposit(&pool_conf, &pool_reserves_after0, &pool_state_after0, &false);

            // Series of random swaps:
            let mut pool_reserves_swaps0 = pool_reserves_after1.clone();
            let mut pool_state_swaps0 = pool_state_after1.clone();
            for _ in 0..10 {
                (pool_reserves_swaps0, pool_state_swaps0, _) =
                    random_swap(&pool_conf, &pool_reserves_swaps0, &pool_state_swaps0);
            }

            // First LP redeem:
            let (pool_reserves_after_redeem0, pool_state_after_redeem0, redeemed_amounts0, req_lp0) =
                random_redeem(&pool_conf, &pool_reserves_swaps0, &pool_state_swaps0, &true);

            // Third LP deposit:
            let (pool_reserves_after2, pool_state_after2, deposited_amounts2, rec_lp2) = random_deposit(
                &pool_conf,
                &pool_reserves_after_redeem0,
                &pool_state_after_redeem0,
                &true,
            );

            // Series of random swaps:
            let mut pool_reserves_swaps1 = pool_reserves_after2.clone();
            let mut pool_state_swaps1 = pool_state_after2.clone();
            for _ in 0..10 {
                (pool_reserves_swaps1, pool_state_swaps1, _) =
                    random_swap(&pool_conf, &pool_reserves_swaps1, &pool_state_swaps1);
            }

            // Second LP redeem:
            let (pool_reserves_after_redeem1, pool_state_after_redeem1, redeemed_amounts1, req_lp1) =
                random_redeem(&pool_conf, &pool_reserves_swaps1, &pool_state_swaps1, &false);

            // Third LP redeem:
            let (pool_reserves_after_redeem2, pool_state_after_redeem2, redeemed_amounts2, req_lp2) =
                random_redeem(
                    &pool_conf,
                    &pool_reserves_after_redeem1,
                    &pool_state_after_redeem1,
                    &true,
                );
        }
    }
}
