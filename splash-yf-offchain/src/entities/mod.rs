pub mod auth_manager;
pub mod buffer_wallet;
pub mod funding_box;
pub mod gauge;
pub mod harvest_order;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferWalletSplashTokenIncrease(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferWalletSplashTokenDecrease(pub u64);

pub enum BufferWalletSplashBalanceChange {
    Increase(u64),
    Decrease(u64),
}

impl BufferWalletSplashBalanceChange {
    pub fn amount(&self) -> u64 {
        match self {
            BufferWalletSplashBalanceChange::Increase(amount)
            | BufferWalletSplashBalanceChange::Decrease(amount) => *amount,
        }
    }

    pub fn from_diff(gauge_input_amount: u64, gauge_output_amount: u64) -> Self {
        if gauge_input_amount > gauge_output_amount {
            BufferWalletSplashBalanceChange::Increase(gauge_input_amount - gauge_output_amount)
        } else {
            BufferWalletSplashBalanceChange::Decrease(gauge_output_amount - gauge_input_amount)
        }
    }
}
