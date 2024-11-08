use crate::analytics::LaunchType::{Common, Fair};
use reqwest::Error;
use spectrum_cardano_lib::Token;
use std::time::Duration;
use log::{error, info};

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

        info!("test: {}", format!("{}.{}", token.0.to_hex(), hex::encode(token.1.as_bytes())));
        info!("self.url: {}", self.url);

        let analytics_response = self
            .client
            .get(self.url.as_str())
            .query(&request_params)
            .send()
            .await?;

        let parsed_pool_info = match analytics_response.json::<PoolResponse>().await {
            Ok(resp) => {
                resp
            }
            Err(err) => {
                error!("Error {:?} during request: {:?}", err, request_params);
                return Err(err);
            }
        };

        Ok(TokenPoolInfo {
            launch_type: parsed_pool_info.pool.launch_type,
            created_on: Duration::from_secs(parsed_pool_info.metrics.created_on),
        })
    }
}

mod tests {
    use log::error;
    use crate::analytics::PoolResponse;

    #[tokio::test]
    async fn parse_fee_switch_datum_mainnet() {
        let client = reqwest::Client::new();

        let request_params = vec![(
            "asset",
            format!("{}.{}", "2b3bf22efec7742d2a193d5d1547f14d0f5f33e1da3a7eaa2ed13ce9", "41444120475245415420414741494e"),
        )];

        let analytics_response = client
            .get("https://api5.splash.trade/platform-api/v2/snekfun/pool/overview/by")
            .query(&request_params)
            .send()
            .await
            .unwrap();

        let a = match analytics_response.json::<PoolResponse>().await {
            Ok(resp) => {
                Some(resp)
            }
            Err(err) => {
                println!("Error {:?} during request: {:?}", err, request_params);
                None
            }
        };

        println!("{:?}", a);

        assert_eq!(1, 2)
    }
}
