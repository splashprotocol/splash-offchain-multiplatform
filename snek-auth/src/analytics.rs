use crate::analytics::LaunchType::{Common, Fair};
use cached::{Cached, SizedCache};
use reqwest::Error;
use spectrum_cardano_lib::Token;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

#[derive(serde::Deserialize, serde::Serialize, Debug, Copy, Clone)]
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

#[derive(serde::Deserialize, serde::Serialize, Debug, Copy, Clone)]
#[serde(rename_all = "camelCase")]
struct PoolInfo {
    launch_type: LaunchType,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Copy, Clone)]
#[serde(rename_all = "camelCase")]
struct MetricsResponse {
    created_on: u64,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Copy, Clone)]
struct PoolResponse {
    pool: PoolInfo,
    metrics: MetricsResponse,
}

#[derive(Copy, Clone)]
pub struct TokenPoolInfo {
    pub launch_type: LaunchType,
    pub created_on: Duration,
}

#[derive(Clone)]
pub struct Analytics {
    url: String,
    client: reqwest::Client,
    cache: Arc<tokio::sync::Mutex<SizedCache<Token, TokenPoolInfo>>>,
}

impl Analytics {
    pub fn new(url: String, cache_size: usize) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
            cache: Arc::new(tokio::sync::Mutex::new(SizedCache::with_size(cache_size))),
        }
    }

    pub async fn get_token_pool_info(&self, token: Token) -> Result<TokenPoolInfo, Error> {
        self.try_retrieve_from_cache(token, |token| {
            Box::pin(async move {
                let request_params = vec![(
                    "asset",
                    format!("{}.{}", token.0.to_hex(), hex::encode(token.1.as_bytes())),
                )];

                let analytics_response = self
                    .client
                    .get(self.url.as_str())
                    .query(&request_params)
                    .send()
                    .await?;

                let parsed_pool_info = analytics_response.json::<PoolResponse>().await?;

                Ok(TokenPoolInfo {
                    launch_type: parsed_pool_info.pool.launch_type,
                    created_on: Duration::from_secs(parsed_pool_info.metrics.created_on),
                })
            })
        })
        .await
    }

    async fn try_retrieve_from_cache<'a, F>(&self, token: Token, get: F) -> Result<TokenPoolInfo, Error>
    where
        F: Fn(Token) -> Pin<Box<dyn Future<Output = Result<TokenPoolInfo, Error>> + 'a>>,
    {
        if let Some(info) = self.cache.lock().await.cache_get(&token) {
            Ok(*info)
        } else {
            let info = get(token).await?;
            self.cache.lock().await.cache_set(token, info);
            Ok(info)
        }
    }
}
