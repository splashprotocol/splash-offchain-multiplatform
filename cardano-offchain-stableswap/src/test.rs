use primitive_types::U512;
use rand::distributions::Standard;
use rand::rngs::StdRng;
use rand::Rng;

/// Prepare random stable3pool parameters and balances.
fn prepare_random_s3pool_state(
    mut rng: StdRng,
    n: u32,
) -> (u32, u32, u32, u64, Vec<U512>, Vec<U512>, Vec<U512>) {
    // Random parameters:
    let a: u32 = rng.gen_range(100..2000);
    let swap_fee_num: u32 = rng.gen_range(0..1_000);
    let protocol_share_num: u32 = rng.gen_range(50..5_000);

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
        .rev()
        .collect::<Vec<U512>>();

    // Total initial reserves (tradable reserves + collected fees):
    let reserves_before = tradable_reserves_before
        .iter()
        .enumerate()
        .map(|(k, &x)| x + collected_protocol_fees[k])
        .collect::<Vec<U512>>();

    // Precision of calculations:
    let precision = reserves_tokens_decimals.iter().max().unwrap().0.as_slice()[0];

    (
        a,
        swap_fee_num,
        protocol_share_num,
        precision,
        reserves_before,
        collected_protocol_fees,
        reserves_tokens_decimals,
    )
}

mod test {
    use primitive_types::U512;
    use rand::distributions::Standard;
    use rand::rngs::StdRng;
    use rand::seq::SliceRandom;
    use rand::{Rng, SeedableRng};

    use crate::stable_swap_amm_actions::{
        liquidity_action, redeem_uniform, swap, LP_EMISSION, LP_NUM_DECIMALS,
    };
    use crate::test::prepare_random_s3pool_state;

    #[test]
    /// Test arbitrary deposit (same logic for uniform deposit).
    fn deposit_test() {
        let n_assets_set: Vec<u32> = vec![3];
        let rng = StdRng::seed_from_u64(42);
        for _ in 0..10000 {
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let (
                a,
                swap_fee_num,
                protocol_share_num,
                precision,
                reserves_before,
                collected_protocol_fees,
                reserves_tokens_decimals,
            ) = prepare_random_s3pool_state(rng.clone(), n);

            let deposited_reserves: Vec<U512> = rand::thread_rng()
                .sample_iter::<u32, Standard>(Standard)
                .take(n.try_into().unwrap())
                .collect::<Vec<_>>()
                .into_iter()
                .enumerate()
                .map(|(k, x)| U512::from(x) * reserves_tokens_decimals[k] + U512::from(x))
                .rev()
                .collect::<Vec<U512>>();

            let reserves_after = reserves_before
                .iter()
                .enumerate()
                .map(|(k, &x)| x + deposited_reserves[k])
                .collect::<Vec<U512>>();
            let lp_amount_before = U512::from(LP_EMISSION)
                * U512::from(10_u64.pow(LP_NUM_DECIMALS))
                - U512::from(u64::MAX) * U512::from(precision);

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
        for _ in 0..10000 {
            let n = *n_assets_set.choose(&mut rand::thread_rng()).unwrap();
            let (
                a,
                swap_fee_num,
                protocol_share_num,
                precision,
                reserves_before,
                collected_protocol_fees,
                reserves_tokens_decimals,
            ) = prepare_random_s3pool_state(rng.clone(), n);

            let desired_reserves: Vec<U512> = rand::thread_rng()
                .sample_iter::<u32, Standard>(Standard)
                .take(n.try_into().unwrap())
                .collect::<Vec<_>>()
                .into_iter()
                .enumerate()
                .map(|(k, x)| U512::from(x) * reserves_tokens_decimals[k] + U512::from(x))
                .rev()
                .collect::<Vec<U512>>();

            let reserves_after = reserves_before
                .iter()
                .enumerate()
                .map(|(k, &x)| x - desired_reserves[k])
                .collect::<Vec<U512>>();

            let lp_amount_before = U512::from(LP_EMISSION)
                * U512::from(10_u64.pow(LP_NUM_DECIMALS))
                - U512::from(u64::MAX) * U512::from(precision);

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
                precision,
                reserves_before,
                collected_protocol_fees,
                reserves_tokens_decimals,
            ) = prepare_random_s3pool_state(rng.clone(), n);

            let supply_lp = U512::from(u64::MAX) * U512::from(precision);
            let lp_amount_before = U512::from(LP_EMISSION)
                * U512::from(10_u64.pow(LP_NUM_DECIMALS))
                - supply_lp
                - U512::from(1);

            redeem_uniform(
                &reserves_before,
                &supply_lp,
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
                protocol_share_num,
                precision,
                reserves_before,
                collected_protocol_fees,
                reserves_tokens_decimals,
            ) = prepare_random_s3pool_state(rng.clone(), n);

            let inds = rand::seq::index::sample(&mut rng, (n - 1) as usize, 2).into_vec();
            let i = *inds.get(0).unwrap();
            let j = *inds.get(1).unwrap();

            let base_amount =
                U512::from(rng.gen_range(precision as u32..u32::MAX)) * reserves_tokens_decimals[i];

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
}
