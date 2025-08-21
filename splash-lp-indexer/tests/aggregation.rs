#[cfg(test)]
mod tests {
    struct Simulation {
        // {index: pool{index: epoch(emission)}
        emission_by_pool_by_epoch: Vec<Vec<u64>>,
        // {index: account{index: epoch{index: pool(share)}}}
        accounts: Vec<Vec<Vec<u64>>>,
    }
    #[derive(Debug)]
    struct Distribution {
        total_emission: u64,
        total_distributed: u64,
        // {index: account{index: pool(amount)}}
        by_account_by_pool: Vec<Vec<u64>>,
        // {index: account(amount)}
        aggregated: Vec<u64>,
    }

    fn iterative_distribution(sim: Simulation) -> Distribution {
        let mut distribution = Distribution {
            total_emission: sim.emission_by_pool_by_epoch.iter().flatten().sum(),
            total_distributed: 0,
            by_account_by_pool: vec![vec![0; sim.emission_by_pool_by_epoch.len()]; sim.accounts.len()],
            aggregated: vec![0; sim.accounts.len()],
        };
        for (acc, epochs) in sim.accounts.iter().enumerate() {
            for (epoch, pools) in epochs.iter().enumerate() {
                for (pool, share_bps) in pools.iter().enumerate() {
                    let pe_in_epoch = sim.emission_by_pool_by_epoch[pool][epoch];
                    let reward_in_pool_in_epoch = pe_in_epoch * share_bps / 10_000;
                    distribution.total_distributed += reward_in_pool_in_epoch;
                    distribution.by_account_by_pool[acc][pool] += reward_in_pool_in_epoch;
                    distribution.aggregated[acc] += reward_in_pool_in_epoch;
                }
            }
        }
        distribution
    }

    #[test]
    fn test_iter_distribution_naive() {
        let sim = Simulation {
            // 4 pools, 3 epochs
            emission_by_pool_by_epoch: vec![vec![1_000_000, 1_000_000, 1_000_000]; 4],
            // 5 accounts, 3 epochs, 4 pools
            accounts: vec![vec![vec![2_000, 2_000, 2_000, 2_000]; 3]; 5],
        };
        let distribution = iterative_distribution(sim);
        dbg!(&distribution);
    }

    #[test]
    fn test_iter_distribution() {
        let sim = Simulation {
            // 4 pools, 3 epochs
            emission_by_pool_by_epoch: vec![
                vec![1_000_000, 800_000, 1_000_000],
                vec![1_000_000, 800_000, 1_000_000],
                vec![1_000_000, 800_000, 1_000_000],
                vec![1_000_000, 1_600_000, 1_000_000],
            ],
            // 5 accounts, 3 epochs, 4 pools
            accounts: vec![
                vec![vec![2_000, 2_000, 2_000, 2_400], vec![2_000, 2_000, 2_000, 2_000], vec![2_000, 2_000, 2_000, 2_000]],
                vec![vec![2_000, 2_000, 2_000, 1_900], vec![2_000, 2_000, 2_000, 2_000], vec![2_000, 2_000, 2_000, 2_000]],
                vec![vec![2_000, 2_000, 2_000, 1_900], vec![2_000, 2_000, 2_000, 2_000], vec![2_000, 2_000, 2_000, 2_000]],
                vec![vec![2_000, 2_000, 2_000, 1_900], vec![2_000, 2_000, 2_000, 2_000], vec![2_000, 2_000, 2_000, 2_000]],
                vec![vec![2_000, 2_000, 2_000, 1_900], vec![2_000, 2_000, 2_000, 2_000], vec![2_000, 2_000, 2_000, 2_000]],
            ],
        };
        let distribution = iterative_distribution(sim);
        dbg!(&distribution);
    }
}
