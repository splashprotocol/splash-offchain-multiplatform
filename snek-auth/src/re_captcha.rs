use log::info;

#[derive(Clone, serde::Deserialize, serde::Serialize, Debug)]
pub struct ReCaptchaToken(pub String);

#[derive(Clone, serde::Deserialize, serde::Serialize, Debug)]
pub struct ReCaptchaSecret(String);

#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct VerificationRequest {
    secret: ReCaptchaSecret,
    response: ReCaptchaToken,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct VerificationResult {
    success: bool,
}

#[derive(Clone)]
pub struct ReCaptcha {
    secret: ReCaptchaSecret,
    client: reqwest::Client,
}

impl ReCaptcha {
    pub fn new(secret: ReCaptchaSecret) -> Self {
        Self {
            secret,
            client: reqwest::Client::new(),
        }
    }

    pub async fn verify(&self, token: ReCaptchaToken) -> bool {
        let req = VerificationRequest {
            secret: self.secret.clone(),
            response: token,
        };
        match self.client.post(URL).form(&req).send().await {
            Err(err) => {
                info!("Verify failed {:?}", err);
                false
            }
            Ok(resp) => resp
                .json::<VerificationResult>()
                .await
                .map(|res| {
                    info!("Verification result {:?}", res);
                    res.success
                })
                .unwrap_or(false),
        }
    }
}

const URL: &str = "https://www.google.com/recaptcha/api/siteverify";
