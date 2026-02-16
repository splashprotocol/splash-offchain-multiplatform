pub mod auth_manager;
pub mod buffer_wallet;
pub mod funding_box;
pub mod gauge;
pub mod harvest_order;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferWalletSplashTokenIncrease(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferWalletSplashTokenDecrease(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
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

    pub fn from_gauge_diff(gauge_input_amount: u64, gauge_output_amount: u64) -> Self {
        if gauge_input_amount > gauge_output_amount {
            BufferWalletSplashBalanceChange::Increase(gauge_input_amount - gauge_output_amount)
        } else {
            BufferWalletSplashBalanceChange::Decrease(gauge_output_amount - gauge_input_amount)
        }
    }

    pub fn from_buffer_wallet_diff(
        buffer_wallet_input_amount: u64,
        buffer_wallet_output_amount: u64,
    ) -> Self {
        if buffer_wallet_input_amount > buffer_wallet_output_amount {
            BufferWalletSplashBalanceChange::Decrease(
                buffer_wallet_input_amount - buffer_wallet_output_amount,
            )
        } else {
            BufferWalletSplashBalanceChange::Increase(
                buffer_wallet_output_amount - buffer_wallet_input_amount,
            )
        }
    }
}
