use std::fmt::{Display, Formatter};

use isahc::http::Uri;
use isahc::{Body, HttpClient, Request, Response};

pub trait SlackHealthAlert {
    fn send_alert(&self, string: &str) -> Result<String, String>;
}

#[derive(Debug, Clone)]
pub struct HealthAlertClient {
    pub client: HttpClient,
    pub base_url: Uri,
    pub assigned_partitions: Vec<u64>,
}

impl HealthAlertClient {
    pub fn new(client: HttpClient, base_url: Uri, assigned_partitions: Vec<u64>) -> Self {
        Self {
            client,
            base_url,
            assigned_partitions,
        }
    }
}

impl SlackHealthAlert for HealthAlertClient {
    fn send_alert(&self, string: &str) -> Result<String, String> {
        let cleaned_string = string.replace("\0", "");

        let body = format!(
            r#"{{"text": "Bot partitions {}. {}"}}"#,
            AsEnumList(&self.assigned_partitions),
            cleaned_string
        );

        let request = Request::post(&self.base_url)
            .header("Content-Type", "application/json")
            .body(body)
            .map_err(|_| "failed to build request".to_string())?;

        let response: Result<Response<Body>, String> = self.client.send(request).map_err(|x| {
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

struct AsEnumList<'a, T>(&'a Vec<T>);

impl<'a, T: Display> Display for AsEnumList<'a, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for x in self.0 {
            f.write_str(format!("{}, ", x).as_str())?;
        }
        Ok(())
    }
}
