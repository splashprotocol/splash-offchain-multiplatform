use async_trait::async_trait;
use cml_chain::certs::Credential;
use cml_core::Slot;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct AccountLocked {
    pub locked_at: Slot,
    pub total_share: u64,
}

pub enum LockAccountRejection<RequestId> {
    NotFound,
    AlreadyLocked(RequestId),
}

#[async_trait]
pub trait Positions<RequestId> {
    async fn lock_account(
        &self,
        request_id: &RequestId,
        account: &Credential,
    ) -> Result<AccountLocked, LockAccountRejection<RequestId>>;
}
