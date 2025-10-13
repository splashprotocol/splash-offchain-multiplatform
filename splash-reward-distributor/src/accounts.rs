use async_trait::async_trait;
use cml_chain::certs::Credential;
use log::error;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use splash_yf_offchain::Epoch;

#[derive(Debug, Deserialize)]
pub struct AccountReward {
    pub accumulated_amount: u64,
    pub latest_epoch_inclusive: Epoch,
}

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
pub struct PositionIndex {
    client: Client,
    api_url: String,
}

impl PositionIndex {
    pub fn new(api_url: String) -> Self {
        Self {
            client: Client::new(),
            api_url,
        }
    }
}

#[async_trait]
impl<RequestId> Accounts<RequestId> for PositionIndex {
    async fn query_account_reward(
        &self,
        account: &Credential,
        from_epoch_inclusive: Epoch,
    ) -> Option<AccountReward> {
        let request_body = json!({
            "account": account,
            "from_epoch_inclusive": from_epoch_inclusive
        });

        let response = self
            .client
            .post(format!("{}/accounts/query-reward", self.api_url))
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(response) => {
                if response.status().is_success() {
                    let account_reward: AccountReward = response.json().await.ok()?;
                    Some(account_reward)
                } else {
                    error!(
                        "Failed to query account reward. Status code: {}",
                        response.status()
                    );
                    None
                }
            }
            Err(e) => {
                error!("Failed to query account reward: {}", e);
                None
            }
        }
    }
}
