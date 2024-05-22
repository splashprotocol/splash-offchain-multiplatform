use async_trait::async_trait;
use isahc::http::Uri;
use isahc::{AsyncBody, AsyncReadResponseExt, HttpClient, Request, Response};

#[async_trait]
pub trait SlackHealthAlert {
    async fn send_alert(&self, string: &str) -> Result<(), String>;
}

#[derive(Clone)]
pub struct HealthAlertClient {
    pub client: HttpClient,
    pub base_url: Uri,
}

impl HealthAlertClient {
    pub fn new(client: HttpClient, base_url: Uri) -> Self {
        Self { client, base_url }
    }
}

#[async_trait]
impl SlackHealthAlert for HealthAlertClient {
    async fn send_alert(&self, string: &str) -> Result<(), String> {
        let body = format!(r#"{{"text": "{}"}}"#, string);

        let request = Request::post(&self.base_url)
            .header("Content-Type", "application/json")
            .body(body)
            .map_err(|_| "failed to build request".to_string())?;

        let mut response: Response<AsyncBody> = self.client.send_async(request).await.map_err(|x| {
            println!("error: {:?}", x.to_string());
            "failed to send slack alert".to_string()
        })?;

        if response.status().is_success() {
            Ok(())
        } else {
            return Err("expected 200 from slack query".into());
        }
    }
}
