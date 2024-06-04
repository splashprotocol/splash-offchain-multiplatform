use async_trait::async_trait;
use isahc::http::Uri;
use isahc::{AsyncBody, AsyncReadResponseExt, Body, HttpClient, Request, Response};

pub trait SlackHealthAlert {
    fn send_alert(&self, string: &str) -> Result<String, String>;
}

#[derive(Debug, Clone)]
pub struct HealthAlertClient {
    pub client: HttpClient,
    pub base_url: Uri,
    pub own_partition_index: u64
}

impl HealthAlertClient {
    pub fn new(client: HttpClient, base_url: Uri, own_partition_index: u64) -> Self {
        Self { client, base_url, own_partition_index}
    }
}

impl SlackHealthAlert for HealthAlertClient {
    fn send_alert(&self, string: &str) -> Result<String, String> {
        let cleaned_string = string.replace("\0", "");

        let body = format!(r#"{{"text": "Bot idx {}. {}"}}"#, self.own_partition_index, cleaned_string);

        let request = Request::post(&self.base_url)
            .header("Content-Type", "application/json")
            .body(body)
            .map_err(|_| "failed to build request".to_string())?;

        let mut response: Result<Response<Body>, String> = self.client.send(request).map_err(|x| {
            println!("error: {:?}", x.to_string());
            "failed to send slack alert".to_string()
        });

        return match response {
            Ok(response) => {
                if response.status().is_success() {
                    Ok("Success".to_string())
                } else {
                    Err("expected 200 from slack query".into())
                }
            }
            Err(_) => Err("error sending to slack".into()),
        };
    }
}
