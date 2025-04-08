#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub base_data_path: String,
    pub endpoints: Vec<String>,
}
