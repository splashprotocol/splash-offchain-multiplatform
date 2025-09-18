use async_trait::async_trait;
use cml_chain::certs::Credential;
use serde::Deserialize;
use splash_yf_offchain::Epoch;

#[derive(Debug, Deserialize)]
pub struct AccountReward {
    pub accumulated_amount: u64,
    pub latest_epoch_inclusive: Epoch,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LockedByAnotherReq<RequestId>(pub RequestId);

#[async_trait]
pub trait Accounts<RequestId> {
    /// Query available `account` reward starting from the given epoch `from_epoch_inclusive`.
    async fn query_account_reward(
        &self,
        account: &Credential,
        from_epoch_inclusive: Epoch,
    ) -> Option<AccountReward>;
}

#[derive(Clone)]
pub struct PositionIndex {}

impl PositionIndex {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl<RequestId> Accounts<RequestId> for PositionIndex {
    async fn query_account_reward(
        &self,
        account: &Credential,
        from_epoch_inclusive: Epoch,
    ) -> Option<AccountReward> {
        todo!("DEX-914")
    }
}
