#[cfg(test)]
mod tests {
    #[derive(Clone)]
    struct Simulation {
        // {index: pool{index: epoch(emission)}
        inflation: Vec<Vec<u64>>,
        // {index: account{index: pool{index: epoch(share)}}}
        shares: Vec<Vec<Vec<u64>>>,
    }
    #[derive(Debug)]
    struct IterativeDistribution {
        total_emission: u64,
        total_distributed: u64,
        // {index: account{index: pool{index: epoch(reward)}}}
        rewards: Vec<Vec<Vec<u64>>>,
    }

    fn iterative_distribution(sim: Simulation) -> IterativeDistribution {
        let mut distribution = IterativeDistribution {
            total_emission: sim.inflation.iter().flatten().sum(),
            total_distributed: 0,
            rewards: sim.shares.clone(),
        };
        for (acc, pools) in sim.shares.iter().enumerate() {
            for (pool, epochs) in pools.iter().enumerate() {
                for (epoch, share_bps) in epochs.iter().enumerate() {
                    let pe_in_epoch = sim.inflation[pool][epoch];
                    let reward_in_pool_in_epoch = pe_in_epoch * share_bps / 10_000;
                    distribution.total_distributed += reward_in_pool_in_epoch;
                    distribution.rewards[acc][pool][epoch] = reward_in_pool_in_epoch;
                }
            }
        }
        distribution
    }

    #[test]
    fn test_iter_distribution_naive() {
        let sim = Simulation {
            // 4 pools, 3 epochs
            inflation: vec![vec![1_000_000, 1_000_000, 1_000_000]; 4],
            // 5 accounts, 4 pools, 3 epochs
            shares: vec![vec![vec![2_000, 2_000, 2_000]; 4]; 5],
        };
        let distribution = iterative_distribution(sim.clone());
        dbg!(&distribution);
    }

    #[test]
    fn test_iter_distribution() {
        let sim = Simulation {
            // 4 pools, 3 epochs
            inflation: vec![
                vec![1_000_000_000, 800_000_000, 1_000_000_000],
                vec![1_000_000_000, 800_000_000, 1_000_000_000],
                vec![1_000_000_000, 800_000_000, 1_000_000_000],
                vec![1_000_000_000, 1_600_000_000, 1_000_000_000],
            ],
            // 5 accounts, 4 pools, 3 epochs
            shares: vec![
                vec![
                    vec![2_000, 4_000, 2_400],
                    vec![2_000, 2_000, 2_000],
                    vec![2_000, 2_000, 2_000],
                    vec![2_000, 2_000, 2_000],
                ],
                vec![
                    vec![2_000, 1_500, 1_900],
                    vec![4_000, 2_000, 2_000],
                    vec![2_000, 2_000, 2_000],
                    vec![2_000, 2_000, 2_000],
                ],
                vec![
                    vec![2_000, 1_500, 1_900],
                    vec![0, 2_000, 2_000],
                    vec![2_000, 2_000, 2_000],
                    vec![2_000, 2_000, 6_000],
                ],
                vec![
                    vec![2_000, 1_500, 1_900],
                    vec![2_000, 2_000, 2_000],
                    vec![2_000, 2_000, 2_000],
                    vec![3_000, 2_000, 0],
                ],
                vec![
                    vec![2_000, 1_500, 1_900],
                    vec![2_000, 2_000, 2_000],
                    vec![2_000, 2_000, 2_000],
                    vec![1_000, 2_000, 0],
                ],
            ],
        };
        let distribution = iterative_distribution(sim.clone());
        dbg!(&distribution);
    }
}
