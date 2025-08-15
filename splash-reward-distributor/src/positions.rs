use async_trait::async_trait;
use cml_chain::certs::Credential;
use cml_core::Slot;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct AccountState {
    pub total_share_bps: u64,
    pub activated_at: Slot,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NotFound;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LockedByAnotherReq<RequestId>(pub RequestId);

#[async_trait]
pub trait Positions<RequestId> {
    async fn query_account(&self, account: &Credential) -> Result<AccountState, NotFound>;
    async fn lock_account(
        &self,
        request_id: &RequestId,
        account: &Credential,
    ) -> Result<(), LockedByAnotherReq<RequestId>>;
}

#[derive(Clone)]
pub struct PositionIndex {}

impl PositionIndex {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl<RequestId> Positions<RequestId> for PositionIndex {
    async fn query_account(&self, account: &Credential) -> Result<AccountState, NotFound> {
        todo!("DEX-914")
    }

    async fn lock_account(
        &self,
        request_id: &RequestId,
        account: &Credential,
    ) -> Result<(), LockedByAnotherReq<RequestId>> {
        todo!("DEX-914")
    }
}
