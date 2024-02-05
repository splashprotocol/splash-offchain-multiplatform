use crate::crypto_pool_data::{PoolConfig, PoolReserves, PoolStateData};
use crate::crypto_pools_amm_actions::{
    liquidity_action, swap, FEE_DENOM, FEE_GAMMA_DENOM, GAMMA_DENOM, LP_EMISSION, PRECISION,
};
use crate::curve_crypto_invariant::calculate_crypto_invariant;
use primitive_types::U512;
use rand::prelude::SliceRandom;
use rand::Rng;
use std::cmp::{min};

pub const MIN_DEPOSIT: u64 = 1_000_000;
pub const MIN_SWAP: u64 = 1_000;
const GAMMA_NUM_MIN: u64 = 1;
const GAMMA_NUM_MAX: u64 = 1_000_000;
const A_MIN: u64 = 1;
const A_MAX: u64 = 100_000;
const SWAP_FEE_NUM_MIN: u64 = 0;
const PROTOCOL_FEE_NUM_MIN: u64 = 0;
const FEE_GAMMA_NUM_MIN: u64 = 0;
const FEE_MID_NUM_MIN: u64 = 0;
const FEE_OUT_NUM_MIN: u64 = 0;

pub fn pool_config_gen() -> PoolConfig {
    let mut rng = rand::thread_rng();
    PoolConfig {
        n_tradable_assets: U512::from(*vec![2, 3, 4].choose(&mut rand::thread_rng()).unwrap()),
        ampl_coeff: U512::from(rng.gen_range(A_MIN..A_MAX)),
        swap_fee_num: U512::from(rng.gen_range(SWAP_FEE_NUM_MIN..FEE_DENOM)),
        protocol_fee_num: U512::from(rng.gen_range(PROTOCOL_FEE_NUM_MIN..FEE_DENOM)),
        gamma_num: U512::from(rng.gen_range(GAMMA_NUM_MIN..GAMMA_NUM_MAX)),
        fee_gamma_num: U512::from(rng.gen_range(FEE_GAMMA_NUM_MIN..FEE_GAMMA_DENOM)),
        fee_mid_num: U512::from(rng.gen_range(FEE_MID_NUM_MIN..FEE_GAMMA_DENOM)),
        fee_out_num: U512::from(rng.gen_range(FEE_OUT_NUM_MIN..FEE_GAMMA_DENOM)),
    }
}
pub fn pool_reserves_gen(conf: &PoolConfig, equal: &bool) -> (PoolReserves, PoolStateData) {
    let mut rng = rand::thread_rng();

    let n = conf.n_tradable_assets.as_u32();

    // Initial balances:
    let balances = if *equal {
        let b_ = U512::from(rng.gen_range(MIN_DEPOSIT..u64::MAX));
        vec![b_; n as usize]
    } else {
        let mut balances_ = vec![];
        for _ in 0..n as usize {
            let b_i = U512::from(rng.gen_range(MIN_DEPOSIT..u64::MAX));
            balances_.push(b_i);
        }
        balances_
    };

    // Initial price vector:
    let mut price_vec = vec![];
    for _ in 0..n as usize {
        let price_i: u32 = rng.gen();
        price_vec.push(U512::from(price_i));
    }

    let precision = U512::from(PRECISION);
    let balances_calc = balances
        .iter()
        .enumerate()
        .map(|(k, &x)| x * precision / price_vec[k])
        .collect::<Vec<U512>>();

    // LP tokens:
    let supply_lp_init = calculate_crypto_invariant(
        &balances_calc,
        &n,
        &conf.ampl_coeff,
        &conf.gamma_num,
        &U512::from(GAMMA_DENOM),
    );
    (
        PoolReserves {
            tradable: balances.clone(),
            lp_tokens: U512::from(LP_EMISSION) - supply_lp_init,
        },
        PoolStateData {
            price_scale: price_vec,
            xcp_profit: U512::from(1),
            xcp_profit_real: U512::from(1),
        },
    )
}

pub fn random_deposit(
    pool_conf: &PoolConfig,
    pool_reserves: &PoolReserves,
    pool_state: &PoolStateData,
    uniform: &bool,
) -> (PoolReserves, PoolStateData, Vec<U512>, U512) {
    let mut rng = rand::thread_rng();

    // Random deposit:
    let deposited_amounts = if !uniform {
        let mut _deposited_amounts = vec![];
        for _ in 0..pool_conf.n_tradable_assets.as_usize() {
            let amount = U512::from(rng.gen_range(MIN_DEPOSIT..u32::MAX as u64));
            _deposited_amounts.push(amount)
        }
        _deposited_amounts
    } else {
        let deposited_amount = U512::from(rng.gen_range(MIN_DEPOSIT..u32::MAX as u64));
        vec![deposited_amount; usize::try_from(pool_conf.n_tradable_assets.as_usize()).unwrap()]
    };

    let pool_tradable_reserves_after = (*pool_reserves.tradable)
        .iter()
        .clone()
        .enumerate()
        .map(|(k, &x)| x + deposited_amounts[k])
        .collect::<Vec<U512>>();

    let (pool_reserves_after, pool_state_after) = liquidity_action(
        pool_reserves,
        &pool_tradable_reserves_after,
        &pool_conf,
        &pool_state,
    );
    let received_lp = pool_reserves.lp_tokens - pool_reserves_after.lp_tokens;
    (
        pool_reserves_after,
        pool_state_after,
        deposited_amounts,
        received_lp,
    )
}

pub fn random_redeem(
    pool_conf: &PoolConfig,
    pool_reserves: &PoolReserves,
    pool_state: &PoolStateData,
    uniform: &bool,
) -> (PoolReserves, PoolStateData, Vec<U512>, U512) {
    let mut rng = rand::thread_rng();

    let tradable_reserves_before = (*pool_reserves).clone().tradable;

    // Random redeem:
    let redeemed_amounts = if !uniform {
        let mut _redeemed_amounts = vec![];
        for i in 0..pool_conf.n_tradable_assets.as_usize() {
            let amount = U512::from(rng.gen_range(MIN_DEPOSIT..tradable_reserves_before[i].as_u64()));
            _redeemed_amounts.push(amount)
        }
        _redeemed_amounts
    } else {
        let max_to_redeem = tradable_reserves_before.iter().min().unwrap().as_u64();
        let redeemed_amount = U512::from(rng.gen_range(MIN_DEPOSIT..max_to_redeem - MIN_DEPOSIT));
        vec![redeemed_amount; usize::try_from(pool_conf.n_tradable_assets.as_usize()).unwrap()]
    };

    let pool_tradable_reserves_after = (*pool_reserves.tradable)
        .iter()
        .clone()
        .enumerate()
        .map(|(k, &x)| x - redeemed_amounts[k])
        .collect::<Vec<U512>>();

    let (pool_reserves_after, pool_state_after) = liquidity_action(
        pool_reserves,
        &pool_tradable_reserves_after,
        &pool_conf,
        &pool_state,
    );
    let required_lp = pool_reserves_after.lp_tokens - pool_reserves.lp_tokens;
    (
        pool_reserves_after,
        pool_state_after,
        redeemed_amounts,
        required_lp,
    )
}

pub fn random_swap(
    pool_conf: &PoolConfig,
    pool_reserves: &PoolReserves,
    pool_state: &PoolStateData,
) -> (PoolReserves, PoolStateData, U512) {
    let mut rng = rand::thread_rng();

    // Random swap:
    let idx = rand::seq::index::sample(&mut rng, pool_conf.n_tradable_assets.as_usize(), 2).into_vec();
    let i = *idx.get(0).unwrap();
    let j = *idx.get(1).unwrap();

    let base_upper_limit = min(
        pool_reserves.tradable[j] * pool_state.price_scale[i] / pool_state.price_scale[j],
        U512::from(u64::MAX),
    );

    let base_amount = U512::from(rng.gen_range(MIN_SWAP..base_upper_limit.as_u64() - 1));

    let (pool_reserves_after, pool_state_after) =
        swap(&i, &j, &base_amount, &pool_reserves, &pool_conf, &pool_state);

    let quote_amount_rec = pool_reserves.tradable[j] - pool_reserves_after.tradable[j];
    (pool_reserves_after, pool_state_after, quote_amount_rec)
}
