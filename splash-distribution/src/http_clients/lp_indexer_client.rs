use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use spectrum_offchain_cardano::data::PoolId;

#[async_trait]
pub trait LpIndexerClient {
    async fn lock_user(&self, account: String) -> Option<LpIndexerAccount>;
    async fn get_user_info(&self, account: String) -> Result<Option<LpIndexerAccount>, String>;
}

type PersonalShare = u64;
type TotalShare = u64;

#[derive(Serialize, Deserialize)]
pub struct LpIndexerAccount {
    pub credential: String,
    // Latest share as (personal_share, total_share)
    pub user_available_splash: u64,
}
