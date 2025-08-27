use crate::position_db::PositionDB;
use cml_chain::certs::Credential;
use splash_yf_offchain::Epoch;

#[derive(Debug)]
pub struct AccountReward {
    pub amount: u64,
    pub latest_epoch_inclusive: Epoch,
}

#[async_trait::async_trait]
pub trait Accounts {
    async fn query_account(&self, cred: Credential, from_epoch_inclusive: Epoch) -> Option<AccountReward>;
}

#[async_trait::async_trait]
impl Accounts for PositionDB {
    async fn query_account(&self, cred: Credential, from_epoch_inclusive: Epoch) -> Option<AccountReward> {
        None
    }
}
