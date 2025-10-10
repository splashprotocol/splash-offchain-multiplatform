pub mod auth_manager;
pub mod buffer_wallet;
pub mod funding_box;
pub mod gauge;
pub mod harvest_order;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplashTokenIncrease(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplashTokenDecrease(pub u64);

pub enum SplashBalanceChange {
    Increase(u64),
    Decrease(u64),
}

impl SplashBalanceChange {
    pub fn amount(&self) -> u64 {
        match self {
            SplashBalanceChange::Increase(amount) | SplashBalanceChange::Decrease(amount) => *amount,
        }
    }

    pub fn from_diff(input: u64, output: u64) -> Self {
        if input > output {
            SplashBalanceChange::Increase(input - output)
        } else {
            SplashBalanceChange::Decrease(output - input)
        }
    }
}
