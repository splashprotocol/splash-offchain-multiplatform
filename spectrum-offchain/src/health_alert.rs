use async_trait::async_trait;
use isahc::{AsyncBody, AsyncReadResponseExt, Body, HttpClient, Request, Response};
use isahc::http::Uri;

pub trait SlackHealthAlert {
    fn send_alert(&self, string: &str) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct HealthAlertClient {
    pub client: HttpClient,
    pub base_url: Uri,
}

impl HealthAlertClient {
    pub fn new(client: HttpClient, base_url: Uri) -> Self {
        Self { client, base_url }
    }
}

impl SlackHealthAlert for HealthAlertClient {
    fn send_alert(&self, string: &str) -> Result<(), String> {
        let body = format!(r#"{{"text": "{}"}}"#, string);

        let request = Request::post(&self.base_url)
            .header("Content-Type", "application/json")
            .body(body)
            .map_err(|_| "failed to build request".to_string())?;

        let mut response: Result<Response<Body>, String> = self.client.send(request)
            .map_err(|x| {
                println!("error: {:?}", x.to_string());
                "failed to send slack alert".to_string()
            });

        return match response {
            Ok(response) => {
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err("expected 200 from slack query".into())
                }
            }
            Err(_) => Err("error sending to slack".into())
        }
    }
}