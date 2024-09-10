use crate::analytics::LaunchType::{Common, Fair};
use reqwest::Error;
use spectrum_cardano_lib::Token;
use std::time::Duration;

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
#[serde(from = "String")]
pub enum LaunchType {
    Fair,
    // Analytics return empty string in case of 'common' launch pipeline
    Common,
}

impl From<String> for LaunchType {
    fn from(value: String) -> Self {
        match value.as_str() {
            "fair" => Fair,
            _ => Common,
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct PoolInfo {
    launch_type: LaunchType,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct MetricsResponse {
    created_on: u64,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct PoolResponse {
    pool: PoolInfo,
    metrics: MetricsResponse,
}

#[derive(Clone)]
pub struct TokenPoolInfo {
    pub launch_type: LaunchType,
    pub created_on: Duration,
}

#[derive(Clone)]
pub struct Analytics {
    url: String,
    client: reqwest::Client,
}

impl Analytics {
    pub fn new(url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
        }
    }

    pub async fn get_token_pool_info(&self, token: Token) -> Result<TokenPoolInfo, Error> {
        let request_params = vec![(
            "asset",
            format!("{}.{}", token.0.to_hex(), hex::encode(token.1.as_bytes())),
        )];

        let analytics_response = self
            .client
            .get(self.url.as_str())
            .query(&request_params)
            .send()
            .await
            .unwrap();

        let parsed_pool_info = analytics_response.json::<PoolResponse>().await.unwrap();

        Ok(TokenPoolInfo {
            launch_type: parsed_pool_info.pool.launch_type,
            created_on: Duration::from_secs(parsed_pool_info.metrics.created_on),
        })
    }
}
