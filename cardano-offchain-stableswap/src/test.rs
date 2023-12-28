use primitive_types::U512;
use rand::distributions::Standard;
use rand::rngs::StdRng;
use rand::Rng;

use crate::stable_swap_invariant::calculate_invariant;

/// Prepare random stable3pool parameters and balances.
fn prepare_random_s3pool_state(
    mut rng: StdRng,
    n: u32,
) -> (u32, u32, u32, Vec<U512>, Vec<U512>, Vec<U512>, U512) {
    // Random parameters:
    let a: u32 = rng.gen_range(100..2000);
    let swap_fee_num: u32 = rng.gen_range(1_000..5_000);
    let protocol_share_num: u32 = rng.gen_range(50..5_00);

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
        protocol_share_num,
        reserves_before,
        collected_protocol_fees,
        reserves_tokens_decimals,
        initial_supply_lp,
    )
}

mod test {
    use primitive_types::U512;
    use rand::distributions::Standard;
    use rand::rngs::StdRng;
    use rand::seq::SliceRandom;
    use rand::{Rng, SeedableRng};

    use crate::stable_swap_amm_actions::{
        liquidity_action, redeem_uniform, swap, DENOM, LP_EMISSION, LP_NUM_DECIMALS,
    };
    use crate::stable_swap_invariant::calculate_invariant;
    use crate::test::prepare_random_s3pool_state;

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
                protocol_share_num,
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
                U512::from(LP_EMISSION) * U512::from(10_u64.pow(LP_NUM_DECIMALS)) - initial_supply_lp;

            liquidity_action(
                &reserves_before,
                &reserves_after,
                &lp_amount_before,
                &reserves_tokens_decimals,
                &collected_protocol_fees,
                &swap_fee_num,
                &protocol_share_num,
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
                protocol_share_num,
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
                U512::from(LP_EMISSION) * U512::from(10_u64.pow(LP_NUM_DECIMALS)) - initial_supply_lp;

            liquidity_action(
                &reserves_before,
                &reserves_after,
                &lp_amount_before,
                &reserves_tokens_decimals,
                &collected_protocol_fees,
                &swap_fee_num,
                &protocol_share_num,
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
                U512::from(LP_EMISSION) * U512::from(10_u64.pow(LP_NUM_DECIMALS)) - initial_supply_lp;
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
        for _ in 0..100000 {
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let (
                a,
                swap_fee_num,
                protocol_share_num,
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
                &protocol_share_num,
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
        for _ in 0..1000 {
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let (a, swap_fee_num, protocol_share_num, _, _, reserves_tokens_decimals, _) =
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
                U512::from(LP_EMISSION) * U512::from(10_u64.pow(LP_NUM_DECIMALS)) - initial_supply_lp;

            let (collected_protocol_fees0, lp_amount_before1, _, _, received_lp0) = liquidity_action(
                &reserves_before,
                &reserves_after0,
                &lp_amount_before0,
                &reserves_tokens_decimals,
                &collected_protocol_fees,
                &swap_fee_num,
                &protocol_share_num,
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

            let (collected_protocol_fees1, lp_amount_before2, d1, d2, received_lp1) = liquidity_action(
                &reserves_after0,
                &reserves_after1,
                &lp_amount_before1,
                &reserves_tokens_decimals,
                &collected_protocol_fees0,
                &swap_fee_num,
                &protocol_share_num,
                &a,
                &n,
            );

            // Series of swaps:
            let mut d1_after_swaps: U512 = d1.clone();
            let mut d2_after_swaps: U512 = d2.clone();
            let mut reserves_after_swaps = reserves_after1.clone();
            let mut collected_protocol_fees_after_swaps = collected_protocol_fees1.clone();
            let mut total_volume = U512::from(0);
            for _ in 0..100 {
                let inds = rand::seq::index::sample(&mut rng, n as usize, 2).into_vec();

                let i = *inds.get(0).unwrap();
                let j = *inds.get(1).unwrap();
                let base_amount = U512::from(1000) * reserves_tokens_decimals[i];

                // let base_amount =
                //     U512::from(rng.gen_range(precision..u32::MAX as u64)) * reserves_tokens_decimals[i];

                (
                    reserves_after_swaps,
                    collected_protocol_fees_after_swaps,
                    d1_after_swaps,
                    d2_after_swaps,
                    _,
                ) = swap(
                    &i,
                    &j,
                    &base_amount,
                    &reserves_after_swaps,
                    &collected_protocol_fees_after_swaps,
                    &reserves_tokens_decimals,
                    &swap_fee_num,
                    &protocol_share_num,
                    &a,
                    &n,
                );
                d1_after_swaps = d1_after_swaps;
                d2_after_swaps = d2_after_swaps;
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
                &protocol_share_num,
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

            let usd_profit0 = usd_received0 - usd_deposited0;
            let usd_profit1 = usd_received1 - usd_deposited1;
            let total_usd_profit = usd_after1 - usd_before1;
            let lp_usd_profit = usd_profit0 + usd_profit1;
            assert_eq!(
                (lp_usd_profit * precision / total_usd_profit) / precision,
                ((received_lp0 + received_lp1) * precision
                    / (initial_supply_lp + received_lp0 + received_lp1))
                    / precision
            );
            assert!(
                collected_protocol_fees_usd
                    >= total_volume
                        * U512::from(swap_fee_num)
                        * U512::from(protocol_share_num)
                        * U512::from(90)
                        / (U512::from(DENOM) * U512::from(DENOM) * U512::from(100))
            );
            assert_eq!(
                (usd_received1 * precision / usd_received0) / precision,
                (received_lp1 * precision / received_lp0) / precision
            );
        }
    }
}
