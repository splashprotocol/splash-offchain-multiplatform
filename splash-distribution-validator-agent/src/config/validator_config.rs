
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorConfig {
    pub validator_private_key: String,
    pub host: String,
    pub port: u16,
}