use crate::account::AccountPosition;
use crate::position_db::PositionDB;
use cml_chain::certs::Credential;
use spectrum_offchain_cardano::data::PoolId;

#[async_trait::async_trait]
pub trait Accounts {
    async fn query_account(&self, cred: Credential) -> Option<Vec<(PoolId, AccountPosition)>>;
}

#[async_trait::async_trait]
impl Accounts for PositionDB {
    async fn query_account(&self, cred: Credential) -> Option<Vec<(PoolId, AccountPosition)>> {
        None
    }
}
